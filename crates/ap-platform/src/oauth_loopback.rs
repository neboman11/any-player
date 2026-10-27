//! Short-lived loopback listener for Spotify's authorization callback.
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PORT: u16 = 8989;
pub const CALLBACK_PATH: &str = "/login";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    #[error("OAuth callback port is already in use")]
    PortInUse,
    #[error("OAuth callback listener failed: {0}")]
    Io(#[from] io::Error),
    #[error("OAuth callback timed out")]
    Timeout,
    #[error("OAuth callback cancelled")]
    Cancelled,
    #[error("OAuth state mismatch")]
    StateMismatch,
    #[error("OAuth authorization failed: {0}")]
    Authorization(String),
    #[error("OAuth callback is missing a code")]
    MissingCode,
}

pub struct SignInFlow {
    pub port: u16,
    pub timeout: Duration,
    pub state: String,
}
impl SignInFlow {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            port: DEFAULT_PORT,
            timeout: DEFAULT_TIMEOUT,
            state: state.into(),
        }
    }
    pub async fn bind(self) -> Result<Listening, LoopbackError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, self.port))
            .await
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AddrInUse {
                    LoopbackError::PortInUse
                } else {
                    LoopbackError::Io(e)
                }
            })?;
        Ok(Listening {
            listener,
            state: self.state,
            timeout: self.timeout,
        })
    }
}
pub struct Listening {
    listener: TcpListener,
    state: String,
    timeout: Duration,
}
impl Listening {
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub async fn wait(self, cancel: CancellationToken) -> Result<String, LoopbackError> {
        tokio::select! {
            () = cancel.cancelled() => Err(LoopbackError::Cancelled),
            result = tokio::time::timeout(self.timeout, self.serve()) => result.unwrap_or(Err(LoopbackError::Timeout)),
        }
    }
    async fn serve(&self) -> Result<String, LoopbackError> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            if let Some(result) = self.handle(&mut stream).await? {
                return result;
            }
        }
    }
    async fn handle(
        &self,
        stream: &mut TcpStream,
    ) -> Result<Option<Result<String, LoopbackError>>, LoopbackError> {
        let mut data = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let count = stream.read(&mut buffer).await?;
            if count == 0 {
                return Ok(None);
            }
            data.extend_from_slice(&buffer[..count]);
            if data.len() > 8192 {
                respond(stream, "400 Bad Request", "Invalid request").await?;
                return Ok(None);
            }
            if data.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let line = data.split(|&b| b == b'\n').next().unwrap_or_default();
        let Ok(line) = std::str::from_utf8(line) else {
            respond(stream, "400 Bad Request", "Invalid request").await?;
            return Ok(None);
        };
        let mut parts = line.split_whitespace();
        let (Some(method), Some(target), Some(_)) = (parts.next(), parts.next(), parts.next())
        else {
            respond(stream, "400 Bad Request", "Invalid request").await?;
            return Ok(None);
        };
        if method != "GET" {
            respond(stream, "405 Method Not Allowed", "Method not allowed").await?;
            return Ok(None);
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != CALLBACK_PATH {
            respond(stream, "404 Not Found", "Not found").await?;
            return Ok(None);
        }
        let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
        let value = |key| {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_ref())
        };
        let result = if value("state") != Some(self.state.as_str()) {
            Err(LoopbackError::StateMismatch)
        } else if let Some(error) = value("error") {
            Err(LoopbackError::Authorization(error.to_owned()))
        } else if let Some(code) = value("code").filter(|v| !v.is_empty()) {
            Ok(code.to_owned())
        } else {
            Err(LoopbackError::MissingCode)
        };
        if result.is_ok() {
            respond(
                stream,
                "200 OK",
                "Sign-in complete. You may close this window.",
            )
            .await?;
        } else {
            respond(
                stream,
                "400 Bad Request",
                "Sign-in failed. Return to Any Player and try again.",
            )
            .await?;
        }
        Ok(Some(result))
    }
}
async fn respond(stream: &mut TcpStream, status: &str, message: &str) -> io::Result<()> {
    let body = format!("<!doctype html><html><body>{message}</body></html>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn request(addr: SocketAddr, target: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }
    #[tokio::test]
    async fn socket_decodes_and_validates_callback() {
        let mut flow = SignInFlow::new("hello state");
        flow.port = 0;
        let listener = flow.bind().await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(listener.wait(CancellationToken::new()));
        assert!(request(addr, "/other").await.contains("404 Not Found"));
        assert!(
            request(addr, "/login?state=hello+state&code=a%2Bb")
                .await
                .contains("200 OK")
        );
        assert_eq!(task.await.unwrap().unwrap(), "a+b");
    }
    #[tokio::test]
    async fn wrong_state_and_cancellation() {
        let mut flow = SignInFlow::new("expected");
        flow.port = 0;
        let listener = flow.bind().await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(listener.wait(CancellationToken::new()));
        assert!(
            request(addr, "/login?state=wrong&code=x")
                .await
                .contains("400 Bad Request")
        );
        assert!(matches!(
            task.await.unwrap(),
            Err(LoopbackError::StateMismatch)
        ));
        let mut flow = SignInFlow::new("x");
        flow.port = 0;
        let listener = flow.bind().await.unwrap();
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            listener.wait(token).await,
            Err(LoopbackError::Cancelled)
        ));
    }
    #[tokio::test]
    async fn timeout_and_provider_error_are_reported() {
        let mut flow = SignInFlow::new("x");
        flow.port = 0;
        flow.timeout = Duration::from_millis(10);
        assert!(matches!(
            flow.bind()
                .await
                .unwrap()
                .wait(CancellationToken::new())
                .await,
            Err(LoopbackError::Timeout)
        ));
        let mut flow = SignInFlow::new("x");
        flow.port = 0;
        let listener = flow.bind().await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(listener.wait(CancellationToken::new()));
        assert!(
            request(addr, "/login?state=x&error=access_denied")
                .await
                .contains("400 Bad Request")
        );
        assert!(
            matches!(task.await.unwrap(), Err(LoopbackError::Authorization(error)) if error == "access_denied")
        );
    }
    #[tokio::test]
    async fn occupied_port_is_distinct() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut flow = SignInFlow::new("x");
        flow.port = listener.local_addr().unwrap().port();
        assert!(matches!(flow.bind().await, Err(LoopbackError::PortInUse)));
    }
}

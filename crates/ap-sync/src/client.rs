//! HTTP and WebSocket transport. JSON numbers retain their original wire literals.
use crate::{Result, SyncError, field, object};
use ap_model::{json::JsonElement, signatures};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{client::IntoClientRequest, http::HeaderValue},
};

#[derive(Clone, PartialEq, Eq)]
pub struct SyncTarget {
    pub server_target: String,
    pub auth_token: String,
}
impl std::fmt::Debug for SyncTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncTarget")
            .field("server_target", &self.server_target)
            .field("auth_token", &"[redacted]")
            .finish()
    }
}
impl SyncTarget {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            server_target: signatures::normalize_server_base_url(url),
            auth_token: signatures::normalize_auth_token(token),
        }
    }
    pub fn key(&self) -> String {
        signatures::base_target(&self.server_target, &self.auth_token)
    }
}
#[derive(Clone, Debug)]
pub struct SyncResponse {
    pub raw: String,
    pub data: JsonElement,
}
impl SyncResponse {
    fn parse(raw: String) -> Result<Self> {
        let data = JsonElement::parse(&raw)?;
        Ok(Self { raw, data })
    }
}
/// A `state_updated` socket event. Fields are read like Android's lenient primitive accessors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncUpdate {
    pub namespace: String,
    pub version: Option<i64>,
    pub source_client_id: Option<String>,
}
impl SyncUpdate {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let json = JsonElement::parse(text).ok()?;
        json.as_object()?;
        if field(&json, "event_type").content_or_null().as_deref() != Some("state_updated") {
            return None;
        }
        Some(Self {
            namespace: field(&json, "namespace").content_or_null()?.into_owned(),
            version: field(&json, "version").as_long_lenient(),
            source_client_id: field(&json, "source_client_id")
                .content_or_null()
                .map(std::borrow::Cow::into_owned),
        })
    }
}
pub type SyncSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[derive(Clone)]
pub struct SyncClient {
    http: reqwest::Client,
}
impl Default for SyncClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}
impl SyncClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
    fn request(
        &self,
        method: reqwest::Method,
        target: &SyncTarget,
        path: &str,
    ) -> Result<reqwest::RequestBuilder> {
        if target.server_target.is_empty() {
            return Err(SyncError::TargetNotSet);
        }
        let request = self
            .http
            .request(method, format!("{}{path}", target.server_target));
        Ok(if target.auth_token.is_empty() {
            request
        } else {
            request.bearer_auth(&target.auth_token)
        })
    }
    async fn get(&self, target: &SyncTarget, path: &str) -> Result<Option<SyncResponse>> {
        let response = self
            .request(reqwest::Method::GET, target, path)?
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        let response = SyncResponse::parse(response.error_for_status()?.text().await?)?;
        if response.data.as_object().is_none() {
            return Err(SyncError::InvalidResponse);
        }
        Ok(Some(response))
    }
    pub async fn snapshot(
        &self,
        target: &SyncTarget,
        since_version: Option<i64>,
    ) -> Result<Option<SyncResponse>> {
        self.get(
            target,
            &since_version.map_or_else(
                || "/v1/snapshot".to_owned(),
                |v| format!("/v1/snapshot?since_version={v}"),
            ),
        )
        .await
    }
    pub async fn app_state(&self, target: &SyncTarget) -> Result<Option<SyncResponse>> {
        let Some(mut response) = self.get(target, "/v1/state/app-state").await? else {
            return Ok(None);
        };
        let JsonElement::Object(mut wire) =
            std::mem::replace(&mut response.data, JsonElement::Null)
        else {
            return Ok(None);
        };
        let Some(data @ JsonElement::Object(_)) = wire.shift_remove("data") else {
            return Ok(None);
        };
        let mut wrapped = indexmap::IndexMap::new();
        if let Some(version) = wire.shift_remove("version") {
            wrapped.insert("version".to_owned(), version);
        }
        wrapped.insert("app_state".to_owned(), data);
        response.data = JsonElement::Object(wrapped);
        Ok(Some(response))
    }
    pub async fn push(
        &self,
        target: &SyncTarget,
        namespace: &str,
        client_id: &str,
        data: JsonElement,
    ) -> Result<()> {
        if !matches!(
            namespace,
            "app-state" | "playlists" | "settings" | "provider-configuration"
        ) {
            return Err(SyncError::Namespace(namespace.to_owned()));
        }
        let payload = object([
            ("client_id", JsonElement::String(client_id.to_owned())),
            ("data", data),
        ]);
        self.request(
            reqwest::Method::PUT,
            target,
            &format!("/v1/state/{namespace}"),
        )?
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string())
        .send()
        .await?
        .error_for_status()?;
        Ok(())
    }
    pub async fn updates(&self, target: &SyncTarget) -> Result<SyncSocket> {
        if target.server_target.is_empty() {
            return Err(SyncError::TargetNotSet);
        }
        let url = target.server_target.replacen("http", "ws", 1) + "/v1/ws";
        let mut request = url.into_client_request()?;
        if !target.auth_token.is_empty() {
            let mut auth = HeaderValue::from_str(&format!("Bearer {}", target.auth_token))
                .map_err(|_| SyncError::Authorization)?;
            auth.set_sensitive(true);
            request.headers_mut().insert("Authorization", auth);
        }
        Ok(tokio_tungstenite::connect_async(request).await?.0)
    }
}
pub(crate) async fn next_update(socket: &mut SyncSocket) -> Result<Option<SyncUpdate>> {
    while let Some(message) = socket.next().await {
        let message = message?;
        if message.is_close() {
            return Ok(None);
        }
        if let Ok(text) = message.to_text()
            && let Some(event) = SyncUpdate::parse(text)
        {
            return Ok(Some(event));
        }
    }
    Ok(None)
}

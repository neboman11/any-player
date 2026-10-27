//! CPAL 0.16 caches its WASAPI enumerator globally, but its COM apartment is
//! thread-local. Keep every native call and stream on one process-lifetime thread:
//! short-lived decoder/UI threads must never invalidate that cached enumerator.
use super::{AudioSink, LocalError, NativeCpalSink, OutputFormat, Render, SinkStream};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, atomic::AtomicBool, mpsc};

type Reply<T> = mpsc::SyncSender<T>;
enum Command {
    Devices(Reply<Result<Vec<String>, LocalError>>),
    Default(Reply<Option<String>>),
    Format(Option<String>, Reply<Result<OutputFormat, LocalError>>),
    Start(
        Option<String>,
        Render,
        Arc<AtomicBool>,
        Reply<Result<u64, LocalError>>,
    ),
    Stop(u64, Reply<()>),
}

fn owner() -> &'static mpsc::Sender<Command> {
    static OWNER: LazyLock<mpsc::Sender<Command>> = LazyLock::new(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ap-wasapi".into())
            .spawn(move || {
                let sink = NativeCpalSink;
                let mut streams = HashMap::new();
                let mut next_id = 0_u64;
                while let Ok(command) = rx.recv() {
                    match command {
                        Command::Devices(reply) => {
                            let _ = reply.send(sink.devices());
                        }
                        Command::Default(reply) => {
                            let _ = reply.send(sink.default_device_name());
                        }
                        Command::Format(name, reply) => {
                            let _ = reply.send(sink.format(name.as_deref()));
                        }
                        Command::Start(name, render, lost, reply) => {
                            let result = sink.start(name.as_deref(), render, lost).map(|stream| {
                                let id = next_id;
                                next_id += 1;
                                streams.insert(id, stream);
                                id
                            });
                            if let Err(mpsc::SendError(Ok(id))) = reply.send(result) {
                                streams.remove(&id);
                            }
                        }
                        Command::Stop(id, reply) => {
                            streams.remove(&id);
                            let _ = reply.send(());
                        }
                    }
                }
            })
            .expect("could not start WASAPI owner");
        tx
    });
    &OWNER
}

fn request<T>(command: impl FnOnce(Reply<T>) -> Command) -> T {
    let (tx, rx) = mpsc::sync_channel(1);
    owner().send(command(tx)).expect("WASAPI owner stopped");
    rx.recv().expect("WASAPI owner dropped response")
}

#[derive(Default)]
pub struct CpalSink;
struct Stream(u64);
impl SinkStream for Stream {}
impl Drop for Stream {
    fn drop(&mut self) {
        request(|reply| Command::Stop(self.0, reply));
    }
}
impl AudioSink for CpalSink {
    fn devices(&self) -> Result<Vec<String>, LocalError> {
        request(Command::Devices)
    }
    fn default_device_name(&self) -> Option<String> {
        request(Command::Default)
    }
    fn format(&self, name: Option<&str>) -> Result<OutputFormat, LocalError> {
        request(|reply| Command::Format(name.map(str::to_owned), reply))
    }
    fn start(
        &self,
        name: Option<&str>,
        render: Render,
        lost: Arc<AtomicBool>,
    ) -> Result<Box<dyn SinkStream>, LocalError> {
        request(|reply| Command::Start(name.map(str::to_owned), render, lost, reply))
            .map(|id| Box::new(Stream(id)) as Box<dyn SinkStream>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_survives_caller_thread_exit_and_concurrent_callers() {
        // The first caller's COM apartment used to disappear at thread exit,
        // leaving CPAL's process-global enumerator dangling for the next call.
        for _ in 0..3 {
            std::thread::spawn(|| {
                let _ = CpalSink.devices();
                let _ = CpalSink.default_device_name();
            })
            .join()
            .unwrap();
        }
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    let _ = CpalSink.devices();
                    let _ = CpalSink.format(None);
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}

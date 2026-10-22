//! Clocked, device-free sink for headless playback. CaptureSink is preferable for sample tests.
use super::{AudioSink, LocalError, OutputFormat, Render, SinkStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
/// Discards rendered audio at the configured sample rate, without an audio device.
pub struct NullSink {
    pub format: OutputFormat,
}
impl Default for NullSink {
    fn default() -> Self {
        Self {
            format: OutputFormat {
                sample_rate: 48_000,
                channels: 2,
            },
        }
    }
}
struct Stream {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl SinkStream for Stream {}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl AudioSink for NullSink {
    fn devices(&self) -> Result<Vec<String>, LocalError> {
        Ok(vec!["Null".into()])
    }
    fn format(&self, _: Option<&str>) -> Result<OutputFormat, LocalError> {
        Ok(self.format)
    }
    fn start(
        &self,
        _: Option<&str>,
        mut render: Render,
        _: Arc<AtomicBool>,
    ) -> Result<Box<dyn SinkStream>, LocalError> {
        if self.format.sample_rate == 0 || self.format.channels == 0 {
            return Err(LocalError::Output(
                "Null sink requires a nonzero rate and channel count".into(),
            ));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let format = self.format;
        let worker = std::thread::Builder::new()
            .name("ap-null-audio".into())
            .spawn(move || {
                let frames = (format.sample_rate / 100).max(1);
                let mut samples = vec![0.0; frames as usize * usize::from(format.channels)];
                let period =
                    Duration::from_secs_f64(f64::from(frames) / f64::from(format.sample_rate));
                let mut at = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    render(&mut samples);
                    at += period;
                    std::thread::sleep(at.saturating_duration_since(Instant::now()));
                }
            })?;
        Ok(Box::new(Stream {
            stop,
            worker: Some(worker),
        }))
    }
}

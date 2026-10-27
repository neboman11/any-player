#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
mod support;
use ap_model::{BackendKind, SourceType};
use ap_playback::StreamRequest;
use ap_playback::local::http::{AudioCache, HttpSource, RANGE_BYTES};
use ap_playback::{
    Backend, BackendEvent, BackendRun, CaptureSink, ChangeOrigin, ItemKey, LocalBackend, RunItem,
};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use support::{Memory, Resolver, track};
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(bytes: Vec<u8>, range: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/audio", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                // Accepted sockets inherit O_NONBLOCK on macOS/BSD. A partial
                // write followed by WouldBlock would truncate the declared body.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while socket.read(&mut byte).unwrap_or(0) > 0 {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request).into_owned();
                seen.lock().unwrap().push(request.clone());
                let requested = request
                    .lines()
                    .find_map(|s| s.strip_prefix("range: bytes="))
                    .and_then(|s| {
                        let (a, b) = s.split_once('-')?;
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    });
                let (start, end) = if range {
                    requested.unwrap_or((0, bytes.len() - 1))
                } else {
                    (0, bytes.len() - 1)
                };
                if start >= bytes.len() {
                    let _=socket.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let end = end.min(bytes.len() - 1);
                let body = &bytes[start..=end];
                let status = if range {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",
                    body.len(),
                    bytes.len()
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(body);
            }
        });
        Self {
            url,
            stop,
            requests,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn cache() -> AudioCache {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    AudioCache::new(std::env::temp_dir().join(format!(
        "ap-audio-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )))
    .unwrap()
}
fn wav(frames: usize, rate: u32, value: i16) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..frames {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}
fn flac(frames: usize, rate: usize, value: i32) -> Vec<u8> {
    use flacenc::{component::BitRepr, error::Verify};
    let mut config = flacenc::config::Encoder::default();
    config.block_size = 1000;
    let config = config.into_verified().unwrap();
    let samples = vec![value; frames];
    let source = flacenc::source::MemSource::from_samples(&samples, 1, 16, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    let bytes = sink.into_inner();
    let source = symphonia::core::io::MediaSourceStream::new(
        Box::new(Cursor::new(bytes.clone())),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );
    symphonia::default::get_probe()
        .format(
            &symphonia::core::probe::Hint::new(),
            source,
            &symphonia::core::formats::FormatOptions::default(),
            &symphonia::core::meta::MetadataOptions::default(),
        )
        .expect("generated FLAC must probe");
    bytes
}
fn run(urls: &[&str]) -> BackendRun {
    BackendRun {
        kind: BackendKind::Local,
        items: urls
            .iter()
            .enumerate()
            .map(|(i, url)| {
                let mut track = track(&format!("item{i}"), SourceType::Plex);
                track.url = Some((*url).into());
                RunItem::Song {
                    uid: i as u64 + 1,
                    track,
                }
            })
            .collect(),
    }
}
async fn wait_worker() {
    tokio::time::sleep(Duration::from_millis(30)).await;
}
fn setup() -> (LocalBackend, CaptureSink, Arc<Memory>) {
    let sink = CaptureSink::new(8000, 1);
    let memory = Arc::new(Memory::default());
    let backend = LocalBackend::new(
        Arc::new(Resolver),
        memory.clone(),
        cache(),
        Arc::new(sink.clone()),
    )
    .unwrap();
    (backend, sink, memory)
}
async fn pump_until_end(
    sink: &CaptureSink,
    events: &mut tokio::sync::broadcast::Receiver<BackendEvent>,
) -> Vec<BackendEvent> {
    let mut found = vec![];
    for _ in 0..200 {
        wait_worker().await;
        sink.pump(400);
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, BackendEvent::Error { .. }), "{event:?}");
            let ended = matches!(event, BackendEvent::RunEnded { .. });
            found.push(event);
            if ended {
                return found;
            }
        }
    }
    panic!("Audio did not end: {found:?}")
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_read_seek_and_disk_cache() {
    let bytes = (0..RANGE_BYTES * 5)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    let server = Server::new(bytes.clone(), true);
    let request = StreamRequest {
        url: server.url.parse().unwrap(),
        headers: reqwest::header::HeaderMap::default(),
        allow_invalid_certs: false,
    };
    let c = cache();
    let req = request.clone();
    let ca = c.clone();
    let expected = bytes.clone();
    tokio::task::spawn_blocking(move || {
        let mut source = HttpSource::new(req, "PLEX:test".into(), ca).unwrap();
        let mut buffer = vec![0; 100];
        source.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, expected[..100]);
        source
            .seek(SeekFrom::Start(RANGE_BYTES as u64 + 7))
            .unwrap();
        source.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, expected[RANGE_BYTES + 7..RANGE_BYTES + 107]);
        source.seek(SeekFrom::Start(0)).unwrap();
        source.prefetch().unwrap();
    })
    .await
    .unwrap();
    let before = server.requests.lock().unwrap().len();
    tokio::task::spawn_blocking(move || {
        let mut source = HttpSource::new(request, "PLEX:test".into(), c).unwrap();
        let mut all = vec![];
        source.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes);
    })
    .await
    .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), before);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_range_forward_and_backward_seek() {
    let bytes = (0..RANGE_BYTES * 3)
        .map(|i| (i % 239) as u8)
        .collect::<Vec<_>>();
    let server = Server::new(bytes.clone(), false);
    let request = StreamRequest {
        url: server.url.parse().unwrap(),
        headers: reqwest::header::HeaderMap::default(),
        allow_invalid_certs: false,
    };
    tokio::task::spawn_blocking(move || {
        let mut source = HttpSource::new(request, "PLEX:no-range".into(), cache()).unwrap();
        source
            .seek(SeekFrom::Start(RANGE_BYTES as u64 + 100))
            .unwrap();
        let mut buffer = [0; 90];
        source.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, &bytes[RANGE_BYTES + 100..RANGE_BYTES + 190]);
        source.seek(SeekFrom::Start(0)).unwrap();
        source.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, &bytes[..90]);
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wav_eof_position_and_gain_cache() {
    let server = Server::new(wav(8000, 8000, 3277), true);
    let (backend, sink, memory) = setup();
    backend.set_normalization(true).await;
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[&server.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    let found = pump_until_end(&sink, &mut events).await;
    assert!(found.contains(&BackendEvent::ItemStarted {
        load_id: 1,
        key: ItemKey::Song(1)
    }));
    assert_eq!(backend.position_ms(), 1000);
    assert_eq!(backend.duration_ms(), 1000);
    let gain = *memory.gains.lock().unwrap().get("PLEX:item0").unwrap();
    assert!((gain - 1.93).abs() < 0.01);
    let samples = sink.samples();
    assert!(samples.iter().skip(80).any(|s| (*s - 0.193).abs() < 0.001));
    backend.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flac_decode_and_no_range_eof() {
    let server = Server::new(flac(8000, 8000, 6554), false);
    let (backend, sink, _) = setup();
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[&server.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    pump_until_end(&sink, &mut events).await;
    assert_eq!(backend.position_ms(), 1000);
    assert!(sink.samples().iter().any(|s| (*s - 0.2).abs() < 0.001));
    backend.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gapless_boundaries_at_consumed_sample_and_repeat_one() {
    let first = Server::new(wav(1600, 8000, 3277), true);
    let second = Server::new(wav(1600, 8000, 6554), true);
    let (backend, sink, _) = setup();
    let mut events = backend.subscribe();
    backend
        .load(
            9,
            run(&[&first.url, &second.url]),
            0,
            0,
            true,
            ChangeOrigin::User,
        )
        .await;
    wait_worker().await;
    sink.pump(1600);
    wait_worker().await;
    let before = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(!before.contains(&BackendEvent::ItemStarted {
        load_id: 9,
        key: ItemKey::Song(2)
    }));
    let samples = sink.pump(1600);
    assert!((samples[0] - 0.2).abs() < 0.001);
    wait_worker().await;
    assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|e| e
        == BackendEvent::ItemStarted {
            load_id: 9,
            key: ItemKey::Song(2)
        }));
    backend.stop().await;
    backend.set_repeat_one(true).await;
    backend
        .load(10, run(&[&first.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    // Let the decoder refill between callbacks, as a real audio device does.
    // One immediate 3200-frame callback races the worker decoding the repeat.
    let mut repeated = Vec::new();
    for _ in 0..200 {
        wait_worker().await;
        sink.pump(80);
        repeated.extend(std::iter::from_fn(|| events.try_recv().ok()));
        assert!(
            !repeated
                .iter()
                .any(|e| matches!(e, BackendEvent::Error { .. })),
            "{repeated:?}"
        );
        if repeated
            .iter()
            .filter(|e| matches!(e, BackendEvent::ItemStarted { load_id: 10, .. }))
            .count()
            >= 2
        {
            break;
        }
    }
    assert_eq!(
        repeated
            .iter()
            .filter(|e| matches!(e, BackendEvent::ItemStarted { load_id: 10, .. }))
            .count(),
        2
    );
    assert!(
        !repeated
            .iter()
            .any(|e| matches!(e, BackendEvent::RunEnded { load_id: 10 }))
    );
    backend.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seek_pause_volume_and_normalization_ramp() {
    let server = Server::new(wav(16_000, 8000, 3277), true);
    let (backend, sink, _) = setup();
    backend
        .load(1, run(&[&server.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    wait_worker().await;
    sink.pump(800);
    assert_eq!(backend.position_ms(), 100);
    backend.pause().await;
    sink.pump(800);
    assert_eq!(backend.position_ms(), 100);
    backend.seek_to(1000).await;
    backend.play(ChangeOrigin::User).await;
    wait_worker().await;
    sink.pump(800);
    assert_eq!(backend.position_ms(), 1100);
    backend.set_normalization(true).await;
    let ramp = sink.pump(100);
    assert!(ramp[0] < ramp[79]);
    assert!((ramp[99] - 0.193).abs() < 0.001);
    backend.set_volume(50).await;
    let ramp = sink.pump(100);
    assert!(ramp[0] > ramp[79]);
    assert!((ramp[99] - 0.0965).abs() < 0.001);
    backend.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decode_error_is_event_not_hung_playback() {
    let server = Server::new(b"not audio".to_vec(), true);
    let (backend, _, _) = setup();
    let mut events = backend.subscribe();
    backend
        .load(17, run(&[&server.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        BackendEvent::Error {
            load_id: 17,
            key: Some(ItemKey::Song(1)),
            ..
        }
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resamples_when_output_rate_differs() {
    let server = Server::new(wav(16_000, 16000, 3277), true);
    let (backend, sink, _) = setup();
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[&server.url]), 0, 0, true, ChangeOrigin::User)
        .await;
    pump_until_end(&sink, &mut events).await;
    assert_eq!(backend.position_ms(), 1000);
    assert!(sink.samples().iter().any(|s| (*s - 0.1).abs() < 0.001));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_skips_failed_lookahead_after_audible_song_drains() {
    use ap_playback::{Engine, EngineDeps, EngineEvent};
    let first = Server::new(wav(1600, 8000, 3277), true);
    let bad = Server::new(b"invalid audio".to_vec(), true);
    let last = Server::new(wav(1600, 8000, 6554), true);
    let (backend, sink, memory) = setup();
    let backend = Arc::new(backend);
    let engine = Engine::spawn(EngineDeps {
        backends: vec![backend.clone()],
        persistence: memory,
        gate: support::Gate::new(),
    });
    engine.await_restored().await;
    engine.set_volume(100, ChangeOrigin::User);
    let tracks = [&first.url, &bad.url, &last.url]
        .into_iter()
        .enumerate()
        .map(|(i, url)| {
            let mut track = track(&format!("track{i}"), SourceType::Plex);
            track.url = Some(url.clone());
            track
        })
        .collect();
    let mut events = engine.events();
    engine.set_queue(tracks, 0, ChangeOrigin::User, true);
    let mut failed = false;
    let mut last_started = false;
    for _ in 0..100 {
        wait_worker().await;
        sink.pump(400);
        while let Ok(event) = events.try_recv() {
            if let EngineEvent::Error { .. } = event {
                failed = true;
            }
            if let EngineEvent::ItemStarted { order_pos: 2, .. } = event {
                last_started = true;
            }
        }
        if failed && last_started {
            break;
        }
    }
    assert!(failed && last_started);
    assert_eq!(
        engine.queue().borrow().current().unwrap().track.id,
        "track2"
    );
    assert!(
        sink.samples()
            .iter()
            .take(1600)
            .skip(80)
            .any(|s| (*s - 0.1).abs() < 0.001)
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gain_analysis_uses_first_six_seconds_even_when_restoring_a_seek() {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..64_000 {
            writer
                .write_sample(if i < 48_000 { 3277_i16 } else { 16_384_i16 })
                .unwrap();
        }
        writer.finalize().unwrap();
    }
    let server = Server::new(cursor.into_inner(), true);
    let (backend, sink, memory) = setup();
    backend.set_normalization(true).await;
    backend
        .load(1, run(&[&server.url]), 0, 7000, true, ChangeOrigin::User)
        .await;
    wait_worker().await;
    let samples = sink.pump(800);
    assert!((memory.gains.lock().unwrap()["PLEX:item0"] - 1.93).abs() < 0.01);
    assert!((samples[799] - 0.965).abs() < 0.001);
    assert_eq!(backend.position_ms(), 7100);
    backend.stop().await;
}

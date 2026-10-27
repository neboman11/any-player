//! VoiceProcessor parity and properties.
//!
//! `data/voice_processor_golden.json` was produced once by running Android's unmodified
//! `VoiceProcessor.kt` on the JVM over the deterministic signal from [`speech`] (identical
//! xorshift32 generator on both sides); it records the output length, peak, loudness and every
//! `stride`-th output sample.

// The signal generator mirrors Kotlin `Int` arithmetic, and sample checks compare exact values.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use std::f64::consts::PI;

use ap_dj::voice_processor::{
    CEILING_DB, TARGET_LUFS, integrated_loudness, normalize_wave_file, process, read_wav, write_wav,
};
use serde::Deserialize;

struct Rng(i32);

impl Rng {
    fn next(&mut self) -> i32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= ((x as u32) >> 17) as i32;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn next_int(&mut self, n: i32) -> i32 {
        ((self.next() as u32 >> 1) as i32) % n
    }
    fn next_double(&mut self) -> f64 {
        f64::from(self.next() as u32 >> 8) / 16_777_216.0
    }
}

/// Voiced harmonic syllables with noisy consonants and pauses, bracketed by silence
/// (Android `VoiceProcessorTest.syntheticSpeech` with a portable generator).
fn speech(fs: i32, scale: f64, seconds: i32, seed: i32) -> Vec<f32> {
    let mut random = Rng(seed);
    let mut samples = vec![0.0f32; (fs * seconds) as usize];
    let mut t = fs / 4;
    while t < samples.len() as i32 - fs / 4 {
        let length = fs / 5 + random.next_int(fs / 10);
        if t + length > samples.len() as i32 {
            break;
        }
        let f0 = 110.0 + f64::from(random.next_int(40));
        for i in 0..length {
            let envelope = (PI * f64::from(i) / f64::from(length)).sin();
            let mut voiced = 0.0;
            for h in 1..=8 {
                voiced += (2.0 * PI * f0 * f64::from(h) * f64::from(i) / f64::from(fs)).sin()
                    / f64::from(h);
            }
            let consonant = if i < length / 8 {
                random.next_double() * 0.8 - 0.4
            } else {
                0.0
            };
            samples[(t + i) as usize] = (scale * 0.3 * envelope * (voiced + consonant)) as f32;
        }
        t += length + fs / 20 + random.next_int(fs / 10);
    }
    samples
}

fn loudness_of(samples: &[f32], fs: u32) -> f64 {
    let x: Vec<f64> = samples.iter().map(|&s| f64::from(s)).collect();
    integrated_loudness(&x, f64::from(fs))
}

fn ceiling() -> f32 {
    10f64.powf(CEILING_DB / 20.0) as f32
}

#[derive(Deserialize)]
struct Golden {
    stride: usize,
    cases: Vec<GoldenCase>,
}

#[derive(Deserialize)]
struct GoldenCase {
    sample_rate: i32,
    scale: f64,
    level: f32,
    seed: i32,
    seconds: i32,
    input_loudness: f64,
    output_loudness: f64,
    output_len: usize,
    output_peak: f32,
    samples: Vec<f32>,
}

#[test]
fn matches_android_golden_outputs() {
    let golden: Golden =
        serde_json::from_str(include_str!("data/voice_processor_golden.json")).unwrap();
    for case in &golden.cases {
        let fs = case.sample_rate as u32;
        let input = speech(case.sample_rate, case.scale, case.seconds, case.seed);
        assert!(
            (loudness_of(&input, fs) - case.input_loudness).abs() < 1e-9,
            "input differs for seed {}",
            case.seed
        );
        let output = process(&input, fs, case.level);
        assert_eq!(output.len(), case.output_len, "seed {}", case.seed);
        let peak = output.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            (peak - case.output_peak).abs() < 1e-6,
            "peak {peak} vs {}",
            case.output_peak
        );
        assert!(
            (loudness_of(&output, fs) - case.output_loudness).abs() < 1e-6,
            "seed {}",
            case.seed
        );
        for (k, &expected) in case.samples.iter().enumerate() {
            let actual = output[k * golden.stride];
            assert!(
                (actual - expected).abs() <= 1e-6,
                "seed {} sample {}: {actual} vs {expected}",
                case.seed,
                k * golden.stride
            );
        }
    }
}

#[test]
fn integrated_loudness_matches_bs1770_full_scale_sine() {
    for fs in [24_000u32, 48_000] {
        let sine: Vec<f64> = (0..fs * 5)
            .map(|i| (2.0 * PI * 997.0 * f64::from(i) / f64::from(fs)).sin())
            .collect();
        assert!((integrated_loudness(&sine, f64::from(fs)) + 3.01).abs() < 0.1);
    }
    assert_eq!(
        integrated_loudness(&vec![0.0; 48_000], 48_000.0),
        f64::NEG_INFINITY
    );
}

#[test]
fn speech_reaches_target_within_half_lu_under_ceiling_with_padded_edges() {
    for fs in [16_000u32, 24_000, 44_100, 48_000] {
        for scale in [0.02, 0.3, 0.9, 4.0] {
            let input = speech(fs as i32, scale, 4, 7);
            let output = process(&input, fs, 1.0);
            let loudness = loudness_of(&output, fs);
            assert!(
                (loudness - TARGET_LUFS).abs() <= 0.5,
                "fs {fs} scale {scale}: {loudness} LUFS"
            );
            assert!(
                output.iter().all(|s| s.abs() <= ceiling() + 1e-6),
                "fs {fs} scale {scale} over ceiling"
            );
            let lead = (fs * 50 / 1000) as usize;
            let tail = (fs * 200 / 1000) as usize;
            assert!(output[..lead].iter().all(|&s| s == 0.0));
            assert!(output[output.len() - tail..].iter().all(|&s| s == 0.0));
            // Trim keeps 20 ms around the first/last audible sample, then the pads are added.
            let threshold = 10f64.powf(-50.0 / 20.0) as f32;
            let first = input.iter().position(|s| s.abs() > threshold).unwrap();
            let last = input.iter().rposition(|s| s.abs() > threshold).unwrap();
            let margin = (fs / 50) as usize;
            let trimmed = (last + margin + 1).min(input.len()) - first.saturating_sub(margin);
            assert_eq!(output.len(), lead + trimmed + tail);
        }
    }
}

#[test]
fn level_trim_shifts_loudness_and_silence_stays_empty() {
    let fs = 24_000;
    let quieter = process(&speech(fs as i32, 0.5, 4, 7), fs, 0.5);
    assert!((loudness_of(&quieter, fs) - (TARGET_LUFS - 6.0)).abs() <= 1.0);
    assert_eq!(process(&vec![0.0; fs as usize], fs, 1.0), [] as [f32; 0]);
    assert_eq!(process(&vec![1e-4; fs as usize], fs, 1.0), [] as [f32; 0]);
    assert_eq!(process(&[], fs, 1.0), [] as [f32; 0]);
}

#[test]
fn wav_round_trip_and_first_channel_decode() {
    let dir = tempfile::tempdir().unwrap();
    let mono = dir.path().join("mono.wav");
    write_wav(&mono, &[0.0, 0.5, -0.5, 1.5, -1.5], 22_050).unwrap();
    let wave = read_wav(&mono).unwrap();
    assert_eq!(wave.sample_rate, 22_050);
    let expected = [
        0.0,
        16_383.0 / 32_768.0,
        -16_383.0 / 32_768.0,
        32_767.0 / 32_768.0,
        -32_768.0 / 32_768.0,
    ];
    assert_eq!(wave.samples, expected.map(|v: f64| v as f32));

    let stereo = dir.path().join("stereo.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&stereo, spec).unwrap();
    for (left, right) in [(100i16, -1i16), (-200, 2), (300, 3)] {
        writer.write_sample(left).unwrap();
        writer.write_sample(right).unwrap();
    }
    writer.finalize().unwrap();
    let wave = read_wav(&stereo).unwrap();
    assert_eq!(
        wave.samples,
        [100.0 / 32_768.0, -200.0 / 32_768.0, 300.0 / 32_768.0].map(|v: f64| v as f32)
    );

    let float = dir.path().join("float.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&float, spec).unwrap();
    for sample in [0.25f32, 0.0, -0.75, 0.0] {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    assert_eq!(read_wav(&float).unwrap().samples, [0.25, -0.75]);

    let byte = dir.path().join("u8.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 8,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&byte, spec).unwrap();
    for sample in [0i8, 64, -128] {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    assert_eq!(read_wav(&byte).unwrap().samples, [0.0, 0.5, -1.0]);
}

#[test]
fn normalize_wave_file_rewrites_mono_pcm16_and_rejects_silence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("break.wav");
    let fs = 24_000;
    write_wav(&path, &speech(fs as i32, 0.3, 3, 9), fs).unwrap();
    assert!(
        normalize_wave_file(&path, 5.0).unwrap(),
        "level is clamped, not rejected"
    );
    let spec = hound::WavReader::open(&path).unwrap().spec();
    assert_eq!(
        (spec.channels, spec.bits_per_sample, spec.sample_rate),
        (1, 16, fs)
    );
    let wave = read_wav(&path).unwrap();
    // Level 5.0 clamps to 2.0 (+6 dB target); the limiter keeps the ceiling.
    assert!(
        wave.samples
            .iter()
            .all(|s| s.abs() <= ceiling() + 1.0 / 32_768.0)
    );

    let silent = dir.path().join("silent.wav");
    write_wav(&silent, &vec![0.0; 1000], fs).unwrap();
    let before = std::fs::read(&silent).unwrap();
    assert!(!normalize_wave_file(&silent, 1.0).unwrap());
    assert_eq!(std::fs::read(&silent).unwrap(), before);

    let garbage = dir.path().join("garbage.wav");
    std::fs::write(&garbage, b"not a wav").unwrap();
    assert!(normalize_wave_file(&garbage, 1.0).is_err());
}

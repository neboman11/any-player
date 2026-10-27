//! Broadcast-style finishing chain for raw TTS output, ported 1:1 from Android
//! `dj/VoiceProcessor.kt` (all math in `f64`, same filters and constants, same evaluation order).
//!
//! The chain trims dead air, shapes the voice with EQ, evens it out with a compressor,
//! normalizes it to [`TARGET_LUFS`] (ITU-R BS.1770 gated integrated loudness) plus the user's
//! level trim, and catches peaks with a lookahead limiter instead of hard clipping.
//!
//! WAV IO mirrors what Android uses around it: sherpa-onnx `WaveReader` (first channel only;
//! 8/16/32-bit integer PCM or 32-bit float) and sherpa-onnx `GeneratedAudio.save` (mono 16-bit PCM).

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::path::Path;

pub const TARGET_LUFS: f64 = -14.0;
pub const CEILING_DB: f64 = -1.0;
const SILENCE_DB: f64 = -50.0;

/// Accepted range of the user's voice level trim (`dj.voice_level`).
pub const LEVEL_RANGE: std::ops::RangeInclusive<f32> = 0.5..=2.0;

/// Processes mono `input` at `sample_rate`. Returns an empty vector when the input is silent.
pub fn process(input: &[f32], sample_rate: u32, level: f32) -> Vec<f32> {
    let trimmed = trim_silence(input, sample_rate);
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut x: Vec<f64> = trimmed.iter().map(|&s| f64::from(s)).collect();
    let fs = f64::from(sample_rate);
    // Rumble/DC out, a little warmth, less boxiness, more presence and air.
    Biquad::high_pass(fs, 80.0, 0.707).run(&mut x);
    Biquad::low_shelf(fs, 150.0, 0.707, 1.5).run(&mut x);
    Biquad::peaking(fs, 300.0, 1.0, -2.5).run(&mut x);
    Biquad::peaking(fs, 3500.0, 0.9, 3.0).run(&mut x);
    if fs > 20_000.0 {
        Biquad::high_shelf(fs, 8000.0, 0.707, 2.0).run(&mut x);
    }
    compress(&mut x, fs, -20.0, 3.0, 6.0, 5.0, 120.0);
    // Limiting peaky speech lowers its loudness, so re-measure and top up; the limiter only
    // ever pulls peaks down, and each pass converges closer to the target.
    let target = TARGET_LUFS + 20.0 * f64::from(level).log10();
    for _ in 0..3 {
        let loudness = integrated_loudness(&x, fs);
        if !loudness.is_finite() {
            continue;
        }
        let gain = db_to_linear(target - loudness);
        for sample in &mut x {
            *sample *= gain;
        }
        limit(&mut x, fs, db_to_linear(CEILING_DB), 3.0, 80.0);
    }
    pad_and_fade(&x, sample_rate)
}

/// ITU-R BS.1770-4 gated integrated loudness in LUFS; `-inf` for silence.
pub fn integrated_loudness(samples: &[f64], fs: f64) -> f64 {
    let mut z = samples.to_vec();
    // K-weighting (pre-filter shelf + RLB high-pass) as RBJ biquads, matching pyloudnorm's
    // any-sample-rate approximation of the standard's 48 kHz coefficients.
    Biquad::high_shelf(fs, 1500.0, std::f64::consts::FRAC_1_SQRT_2, 4.0).run(&mut z);
    Biquad::high_pass(fs, 38.0, 0.5).run(&mut z);
    let block = truncate(0.4 * fs);
    let step = truncate(0.1 * fs);
    let powers: Vec<f64> = if z.len() < block {
        vec![mean_square(&z, 0, z.len())]
    } else {
        (0..=z.len() - block)
            .step_by(step)
            .map(|from| mean_square(&z, from, block))
            .collect()
    };
    let absolute: Vec<f64> = powers
        .into_iter()
        .filter(|&p| block_loudness(p) > -70.0)
        .collect();
    if absolute.is_empty() {
        return f64::NEG_INFINITY;
    }
    let relative_gate = block_loudness(average(absolute.iter().copied())) - 10.0;
    block_loudness(average(
        absolute
            .iter()
            .copied()
            .filter(|&p| block_loudness(p) > relative_gate),
    ))
}

/// Kotlin `Double.toInt()` for the non-negative, in-range values used here.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn truncate(value: f64) -> usize {
    value as usize
}

/// Kotlin `Iterable<Double>.average()`: sequential sum divided by count (NaN when empty).
#[allow(clippy::cast_precision_loss)]
fn average(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(sum, count), v| (sum + v, count + 1));
    sum / count as f64
}

#[allow(clippy::cast_precision_loss)]
fn mean_square(z: &[f64], from: usize, count: usize) -> f64 {
    let mut sum = 0.0;
    for &v in &z[from..from + count] {
        sum += v * v;
    }
    sum / count as f64
}

fn block_loudness(power: f64) -> f64 {
    -0.691 + 10.0 * power.log10()
}

#[allow(clippy::cast_possible_truncation)]
fn trim_silence(input: &[f32], sample_rate: u32) -> &[f32] {
    let threshold = db_to_linear(SILENCE_DB) as f32;
    let Some(first) = input.iter().position(|s| s.abs() > threshold) else {
        return &[];
    };
    let last = input
        .iter()
        .rposition(|s| s.abs() > threshold)
        .unwrap_or(first);
    let margin = (sample_rate / 50) as usize;
    &input[first.saturating_sub(margin)..input.len().min(last + margin + 1)]
}

/// Feed-forward compressor: peak envelope follower driving a soft-knee gain computer.
fn compress(
    x: &mut [f64],
    fs: f64,
    threshold_db: f64,
    ratio: f64,
    knee_db: f64,
    attack_ms: f64,
    release_ms: f64,
) {
    let attack = (-1.0 / (attack_ms / 1000.0 * fs)).exp();
    let release = (-1.0 / (release_ms / 1000.0 * fs)).exp();
    let mut envelope = 0.0;
    for sample in x.iter_mut() {
        let rectified = sample.abs();
        let coefficient = if rectified > envelope {
            attack
        } else {
            release
        };
        envelope = coefficient * envelope + (1.0 - coefficient) * rectified;
        let over = 20.0 * envelope.max(1e-9).log10() - threshold_db;
        let reduction_db = if 2.0 * over <= -knee_db {
            0.0
        } else if 2.0 * over.abs() < knee_db {
            (1.0 / ratio - 1.0) * (over + knee_db / 2.0).powi(2) / (2.0 * knee_db)
        } else {
            (1.0 / ratio - 1.0) * over
        };
        *sample *= db_to_linear(reduction_db);
    }
}

/// Lookahead peak limiter. Each sample's required gain is min-filtered forward over the
/// lookahead window, then averaged over the trailing window: every averaged value comes from
/// windows that all cover the current sample, so the smoothed gain ramps down ahead of a peak
/// yet never exceeds what that peak needs. Recovery uses a one-pole release, which only ever
/// lags below the target, so it stays safe too.
#[allow(clippy::cast_precision_loss)]
fn limit(x: &mut [f64], fs: f64, ceiling: f64, lookahead_ms: f64, release_ms: f64) {
    let window = truncate(lookahead_ms / 1000.0 * fs).max(1);
    let needed: Vec<f64> = x
        .iter()
        .map(|s| 1.0f64.min(ceiling / s.abs().max(1e-12)))
        .collect();
    let minimum = forward_minimum(&needed, window);
    let release = (-1.0 / (release_ms / 1000.0 * fs)).exp();
    let mut running = 0.0;
    let mut gain = 1.0;
    for (i, sample) in x.iter_mut().enumerate() {
        running += minimum[i];
        if i >= window {
            running -= minimum[i - window];
        }
        let averaged = running / (i + 1).min(window) as f64;
        gain = if averaged < gain {
            averaged
        } else {
            release * gain + (1.0 - release) * averaged
        };
        *sample *= gain;
    }
}

/// `out[i] = min(1, needed[i..min(len, i + window)])`, computed with a monotonic deque. The
/// minimum of a set is exact, so this equals Android's direct nested loop.
fn forward_minimum(needed: &[f64], window: usize) -> Vec<f64> {
    let mut out = vec![1.0; needed.len()];
    let mut deque: VecDeque<usize> = VecDeque::with_capacity(window + 1);
    for i in (0..needed.len()).rev() {
        while deque.back().is_some_and(|&k| needed[k] >= needed[i]) {
            deque.pop_back();
        }
        deque.push_back(i);
        while deque.front().is_some_and(|&k| k >= i + window) {
            deque.pop_front();
        }
        out[i] = 1.0f64.min(needed[deque[0]]);
    }
    out
}

/// Short lead-in and tail so the voice doesn't butt against the songs, with fades that keep
/// the trimmed edges click-free.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn pad_and_fade(x: &[f64], sample_rate: u32) -> Vec<f32> {
    let rate = sample_rate as usize;
    let lead = rate * 50 / 1000;
    let tail = rate * 200 / 1000;
    let fade_in = (rate * 10 / 1000).min(x.len()).max(1) as f64;
    let fade_out = (rate * 40 / 1000).min(x.len()).max(1) as f64;
    let mut out = vec![0.0f32; lead + x.len() + tail];
    for (i, &sample) in x.iter().enumerate() {
        let from_end = x.len() - 1 - i;
        let fade = 1.0f64.min((i as f64 / fade_in).min(from_end as f64 / fade_out));
        out[lead + i] = (sample * fade) as f32;
    }
    out
}

fn db_to_linear(db: f64) -> f64 {
    10.0f64.powf(db / 20.0)
}

/// RBJ Audio EQ Cookbook biquad, transposed direct form II.
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

// Kept as the cookbook's single-letter formulas, evaluated in Android's exact order.
#[allow(clippy::many_single_char_names, clippy::manual_midpoint)]
impl Biquad {
    fn run(&self, x: &mut [f64]) {
        let mut s1 = 0.0;
        let mut s2 = 0.0;
        for sample in x.iter_mut() {
            let input = *sample;
            let output = self.b0 * input + s1;
            s1 = self.b1 * input - self.a1 * output + s2;
            s2 = self.b2 * input - self.a2 * output;
            *sample = output;
        }
    }

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    fn high_pass(fs: f64, f0: f64, q: f64) -> Self {
        let w = 2.0 * PI * f0 / fs;
        let alpha = w.sin() / (2.0 * q);
        let c = w.cos();
        Self::normalized(
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    fn peaking(fs: f64, f0: f64, q: f64, gain_db: f64) -> Self {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f0 / fs;
        let alpha = w.sin() / (2.0 * q);
        let c = w.cos();
        Self::normalized(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        )
    }

    fn low_shelf(fs: f64, f0: f64, q: f64, gain_db: f64) -> Self {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f0 / fs;
        let k = 2.0 * a.sqrt() * w.sin() / (2.0 * q);
        let c = w.cos();
        Self::normalized(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }

    fn high_shelf(fs: f64, f0: f64, q: f64, gain_db: f64) -> Self {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f0 / fs;
        let k = 2.0 * a.sqrt() * w.sin() / (2.0 * q);
        let c = w.cos();
        Self::normalized(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        )
    }
}

/// Mono audio as decoded from a WAV file.
#[derive(Debug, Clone, PartialEq)]
pub struct Wave {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("WAV decode/encode failed: {0}")]
    Hound(#[from] hound::Error),
    #[error("unsupported WAV sample format: {bits}-bit {format:?}")]
    Unsupported {
        bits: u16,
        format: hound::SampleFormat,
    },
}

/// Reads a WAV file the way sherpa-onnx `WaveReader.readWave` does: only the first channel is
/// kept; 8/16/32-bit integer PCM and 32-bit float are accepted.
pub fn read_wav(path: &Path) -> Result<Wave, WavError> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        // hound already re-centres unsigned 8-bit samples, so `s / 128` equals sherpa's `u / 128 - 1`.
        (hound::SampleFormat::Int, 8 | 16 | 32) => {
            let scale = f64::from(1u32 << (spec.bits_per_sample - 1));
            first_channel(reader.into_samples::<i32>(), channels, |s| {
                narrow(f64::from(s) / scale)
            })?
        }
        (hound::SampleFormat::Float, 32) => {
            first_channel(reader.into_samples::<f32>(), channels, |s| s)?
        }
        (format, bits) => return Err(WavError::Unsupported { bits, format }),
    };
    Ok(Wave {
        samples,
        sample_rate: spec.sample_rate,
    })
}

#[allow(clippy::cast_possible_truncation)]
fn narrow(value: f64) -> f32 {
    value as f32
}

fn first_channel<S, R: std::io::Read>(
    samples: hound::WavIntoSamples<R, S>,
    channels: usize,
    convert: impl Fn(S) -> f32,
) -> Result<Vec<f32>, WavError>
where
    S: hound::Sample,
{
    let mut out = Vec::with_capacity(samples.size_hint().0 / channels);
    let mut first = None;
    for (index, sample) in samples.enumerate() {
        let sample = sample?;
        if index % channels == 0 {
            first = Some(sample);
        }
        // Like sherpa, a trailing partial frame is dropped.
        if index % channels == channels - 1 {
            out.extend(first.take().map(&convert));
        }
    }
    Ok(out)
}

/// Writes mono 16-bit PCM like sherpa-onnx `GeneratedAudio.save`
/// (`clamp(int32(sample * 32767), -32768, 32767)`).
#[allow(clippy::cast_possible_truncation)]
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), WavError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(((sample * 32767.0) as i32).clamp(-32768, 32767) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Android `VoiceSynthesizer.normalizeWaveFile`: decodes `path`, processes it with `level`
/// clamped to [`LEVEL_RANGE`], and overwrites it in place. Returns `false` (leaving the file
/// untouched) when the processed audio is empty.
pub fn normalize_wave_file(path: &Path, level: f32) -> Result<bool, WavError> {
    let wave = read_wav(path)?;
    let level = level.clamp(*LEVEL_RANGE.start(), *LEVEL_RANGE.end());
    let processed = process(&wave.samples, wave.sample_rate, level);
    if processed.is_empty() {
        return Ok(false);
    }
    write_wav(path, &processed, wave.sample_rate)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_minimum_matches_nested_loop() {
        let needed = [1.0, 0.5, 0.9, 0.2, 0.8, 1.0, 0.7, 0.3, 1.0];
        for window in 1..=needed.len() + 1 {
            let direct: Vec<f64> = (0..needed.len())
                .map(|i| {
                    needed[i..needed.len().min(i + window)]
                        .iter()
                        .fold(1.0f64, |m, &v| m.min(v))
                })
                .collect();
            assert_eq!(forward_minimum(&needed, window), direct, "window {window}");
        }
    }

    #[test]
    fn trim_keeps_twenty_ms_margin() {
        let mut input = vec![0.0f32; 1000];
        input[500] = 0.5;
        input[600] = -0.5;
        let trimmed = trim_silence(&input, 1000);
        assert_eq!(trimmed.len(), 600 - 500 + 1 + 2 * 20);
    }
}

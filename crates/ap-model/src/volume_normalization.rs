//! Per-source volume scaling (Android `VolumeNormalization`, f32 parity) plus the desktop-only
//! per-track loudness gain for local items (design §8.4).

use crate::enums::SourceType;

/// Internal normalization target percentage.
pub const TARGET: i32 = 85;

/// `clamp((0.04 + 0.85 * 0.18) / (0.04 + 0.18), 0.1, 1.0)` computed in `f32` (≈ 0.877).
pub const OTHER_SOURCE_FACTOR: f32 = {
    #[expect(clippy::cast_precision_loss, reason = "85 is exact in f32")]
    let normalized = TARGET as f32 / 100.0;
    let target_rms = 0.04_f32 + normalized * 0.18;
    let max_target_rms = 0.04_f32 + 0.18;
    (target_rms / max_target_rms).clamp(0.1, 1.0)
};

/// The output volume (0..=100) for UI volume `volume` playing a `source` item.
#[must_use]
pub fn effective(volume: i32, source: SourceType, enabled: bool) -> i32 {
    let base = volume.clamp(0, 100);
    if !enabled {
        return base;
    }
    if source == SourceType::Spotify {
        ((base * TARGET + 50) / 100).min(100)
    } else {
        // Java `Math.round(float)` (half up); `base` is non-negative so `round` agrees.
        #[expect(clippy::cast_precision_loss, reason = "base is 0..=100")]
        let scaled = (base as f32 * OTHER_SOURCE_FACTOR).round();
        #[expect(clippy::cast_possible_truncation, reason = "scaled is 0..=100")]
        let rounded = scaled as i32;
        rounded.clamp(0, 100)
    }
}

/// Fewer analyzed samples than this yields gain 1.0.
pub const TRACK_GAIN_MIN_SAMPLES: usize = 4_096;
/// Interleaved samples analyzed (a fixed count, ~6 s of 44.1 kHz stereo; not rate-dependent).
pub const TRACK_GAIN_MAX_SAMPLES: usize = 44_100 * 2 * 6;

fn target_rms_from_percent(target: u32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "0..=100")]
    let normalized = target.min(100) as f32 / 100.0;
    0.04 + (normalized * 0.18)
}

/// Desktop-only loudness gain for a local item, ported verbatim from the legacy desktop.
///
/// `samples` are interleaved f32 samples as decoded; at most [`TRACK_GAIN_MAX_SAMPLES`] are
/// read. Short or near-silent input yields 1.0. Not part of any wire format.
#[must_use]
pub fn track_gain(samples: impl IntoIterator<Item = f32>) -> f32 {
    let mut sum_sq = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut count = 0_usize;
    for s in samples.into_iter().take(TRACK_GAIN_MAX_SAMPLES) {
        let a = s.abs();
        sum_sq += f64::from(a) * f64::from(a);
        peak = peak.max(a);
        count += 1;
    }
    if count < TRACK_GAIN_MIN_SAMPLES {
        return 1.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "count <= 529_200")]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rms of f32 samples fits f32"
    )]
    let rms = (sum_sq / count as f64).sqrt() as f32;
    if rms <= 0.0005 {
        return 1.0;
    }
    let mut gain = (target_rms_from_percent(85) / rms).clamp(0.4, 3.0);
    if peak > 0.0 {
        gain = gain.min(0.98 / peak);
    }
    gain.clamp(0.25, 3.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_clamped_volume() {
        assert_eq!(effective(77, SourceType::Spotify, false), 77);
        assert_eq!(effective(140, SourceType::Jellyfin, false), 100);
        assert_eq!(effective(-5, SourceType::Plex, false), 0);
    }

    #[test]
    fn spotify_uses_integer_target_scaling() {
        assert_eq!(effective(100, SourceType::Spotify, true), 85);
        assert_eq!(effective(50, SourceType::Spotify, true), 43);
        assert_eq!(effective(1, SourceType::Spotify, true), 1);
        assert_eq!(effective(0, SourceType::Spotify, true), 0);
    }

    #[test]
    fn other_sources_use_runtime_factor() {
        assert!((OTHER_SOURCE_FACTOR - 0.877_272_7).abs() < 1e-6);
        assert_eq!(effective(100, SourceType::Jellyfin, true), 88);
        assert_eq!(effective(50, SourceType::Plex, true), 44);
        assert_eq!(effective(10, SourceType::All, true), 9);
        assert_eq!(effective(0, SourceType::Custom, true), 0);
    }

    fn sine(amplitude: f32, len: usize) -> impl Iterator<Item = f32> {
        #[expect(clippy::cast_precision_loss, reason = "test signal")]
        (0..len).map(move |i| amplitude * (i as f32 * 0.05).sin())
    }

    fn reference(amplitude_rms: f32, peak: f32) -> f32 {
        let target = 0.04 + 0.85 * 0.18;
        (target / amplitude_rms)
            .clamp(0.4, 3.0)
            .min(0.98 / peak)
            .clamp(0.25, 3.0)
    }

    #[test]
    fn track_gain_short_or_silent_input_is_unity() {
        assert!(
            (track_gain(std::iter::repeat_n(0.5, TRACK_GAIN_MIN_SAMPLES - 1)) - 1.0).abs()
                < f32::EPSILON
        );
        assert!((track_gain(std::iter::repeat_n(0.0, 100_000)) - 1.0).abs() < f32::EPSILON);
        assert!((track_gain(std::iter::repeat_n(0.0004, 100_000)) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn track_gain_constant_signals() {
        // DC at 0.1: rms = peak = 0.1 → 0.193 / 0.1 = 1.93, under 0.98 / 0.1.
        let g = track_gain(std::iter::repeat_n(0.1, 10_000));
        assert!((g - reference(0.1, 0.1)).abs() < 1e-5, "{g}");
        assert!((g - 1.93).abs() < 1e-4, "{g}");
        // Loud DC at 0.9: 0.193 / 0.9 < 0.4 → clamp to 0.4; peak cap 1.089 does not bind.
        assert!((track_gain(std::iter::repeat_n(-0.9, 10_000)) - 0.4).abs() < 1e-6);
        // Quiet DC at 0.01: 19.3 → 3.0, then peak cap 98 does not bind.
        assert!((track_gain(std::iter::repeat_n(0.01, 10_000)) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn track_gain_sines_follow_rms() {
        for amplitude in [0.05_f32, 0.2, 0.5] {
            let samples: Vec<f32> = sine(amplitude, 200_000).collect();
            let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            #[expect(clippy::cast_precision_loss, reason = "test")]
            #[expect(clippy::cast_possible_truncation, reason = "test")]
            let rms = (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()
                / samples.len() as f64)
                .sqrt() as f32;
            let g = track_gain(samples);
            assert!(
                (g - reference(rms, peak)).abs() < 1e-5,
                "amp {amplitude}: {g}"
            );
        }
        // 0.2 amplitude: rms ≈ 0.1414 → gain ≈ 1.365.
        let g = track_gain(sine(0.2, 200_000));
        assert!((g - 1.365).abs() < 0.01, "{g}");
    }

    #[test]
    fn track_gain_peak_cap_binds_for_spiky_signal() {
        // Mostly quiet (gain would be 3.0) with one full-scale spike: capped at 0.98.
        let mut samples = vec![0.01_f32; 20_000];
        samples[100] = 1.0;
        let g = track_gain(samples);
        assert!((g - 0.98).abs() < 1e-6, "{g}");
    }

    #[test]
    fn track_gain_reads_at_most_max_samples() {
        let quiet = std::iter::repeat_n(0.1_f32, TRACK_GAIN_MAX_SAMPLES);
        let loud_tail = std::iter::repeat_n(0.9_f32, 100_000);
        let g = track_gain(quiet.chain(loud_tail));
        assert!((g - 1.93).abs() < 1e-4, "{g}");
    }
}

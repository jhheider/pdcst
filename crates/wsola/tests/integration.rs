//! Public integration tests: drive `wsola` through its public API exactly as a
//! consumer (podcast-tui) does, and assert the properties that matter -
//! duration scales with tempo, pitch does NOT, output stays finite and bounded,
//! and streaming equals one-shot.

use wsola::{Config, TimeStretch, stretch};

const SR: u32 = 44_100;

/// A mono sine of `freq` Hz for `secs` seconds at 44.1 kHz.
fn sine(freq: f32, secs: f32) -> Vec<f32> {
    let n = (SR as f32 * secs) as usize;
    (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin())
        .collect()
}

/// Estimate the dominant frequency of a mono signal from its zero-crossing rate.
/// For a clean sine this is accurate to a few percent, which is all we need to
/// tell "pitch preserved" from "pitch shifted".
fn dominant_freq(signal: &[f32]) -> f32 {
    // Skip the fade-in/out edges where overlap-add ramps amplitude.
    let a = signal.len() / 8;
    let b = signal.len() - signal.len() / 8;
    if b <= a + 2 {
        return 0.0;
    }
    let seg = &signal[a..b];
    let mut crossings = 0usize;
    for w in seg.windows(2) {
        if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
            crossings += 1;
        }
    }
    // Two zero-crossings per cycle.
    let secs = seg.len() as f32 / SR as f32;
    crossings as f32 / 2.0 / secs
}

fn rms(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }
    (signal.iter().map(|s| s * s).sum::<f32>() / signal.len() as f32).sqrt()
}

#[test]
fn duration_scales_inversely_with_tempo() {
    let input = sine(220.0, 2.0);
    for &tempo in &[0.5f32, 0.75, 1.0, 1.5, 2.0] {
        let out = stretch(&input, SR, 1, tempo).unwrap();
        let expected = input.len() as f32 / tempo;
        let ratio = out.len() as f32 / expected;
        // Within a few percent (boundary frames aside).
        assert!(
            (0.9..=1.1).contains(&ratio),
            "tempo {tempo}: got {} samples, expected ~{expected} (ratio {ratio:.3})",
            out.len()
        );
    }
}

#[test]
fn pitch_is_preserved_across_tempo() {
    let freq = 440.0;
    let input = sine(freq, 2.0);
    let base = dominant_freq(&input);
    assert!(
        (base - freq).abs() / freq < 0.03,
        "sanity: input reads {base} Hz"
    );

    for &tempo in &[0.5f32, 0.75, 1.25, 1.5, 2.0] {
        let out = stretch(&input, SR, 1, tempo).unwrap();
        let f = dominant_freq(&out);
        // The whole point: frequency must NOT move with tempo.
        assert!(
            (f - freq).abs() / freq < 0.05,
            "tempo {tempo}: pitch shifted to {f} Hz (want ~{freq})"
        );
    }
}

#[test]
fn identity_at_tempo_one_preserves_signal() {
    let input = sine(330.0, 1.0);
    let out = stretch(&input, SR, 1, 1.0).unwrap();
    // Length within a frame or two.
    assert!((out.len() as isize - input.len() as isize).unsigned_abs() < 4000);
    // Energy preserved (overlap-add of a Hann COLA reconstructs unit gain).
    let ri = rms(&input);
    let ro = rms(&out);
    assert!(
        (ro / ri - 1.0).abs() < 0.1,
        "rms changed too much: in {ri:.3} out {ro:.3}"
    );
}

#[test]
fn output_is_finite_and_bounded() {
    // A messy mix, not a clean tone.
    let input: Vec<f32> = (0..44_100)
        .map(|i| {
            let t = i as f32 / SR as f32;
            0.5 * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
                + 0.3 * (2.0 * std::f32::consts::PI * 517.0 * t).sin()
        })
        .collect();
    let peak = input.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    for &tempo in &[0.5f32, 1.0, 1.7, 3.0] {
        let out = stretch(&input, SR, 1, tempo).unwrap();
        assert!(
            out.iter().all(|s| s.is_finite()),
            "tempo {tempo}: non-finite output"
        );
        let out_peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            out_peak <= peak * 1.5 + 1e-3,
            "tempo {tempo}: output peak {out_peak} blew past input peak {peak}"
        );
    }
}

#[test]
fn streaming_equals_one_shot() {
    let input = sine(300.0, 1.5);
    let one_shot = stretch(&input, SR, 1, 1.5).unwrap();

    // Feed the same input in irregular chunks, pulling small amounts as we go.
    let mut ts = TimeStretch::new(SR, 1).unwrap();
    ts.set_tempo(1.5);
    let mut streamed = Vec::new();
    let chunks = [17usize, 1024, 4096, 99, 8192, 333];
    let mut pos = 0;
    let mut ci = 0;
    while pos < input.len() {
        let n = chunks[ci % chunks.len()].min(input.len() - pos);
        ts.push(&input[pos..pos + n]);
        pos += n;
        ci += 1;
        streamed.extend(ts.pull(500));
    }
    streamed.extend(ts.flush());

    assert_eq!(
        streamed.len(),
        one_shot.len(),
        "streamed {} vs one-shot {}",
        streamed.len(),
        one_shot.len()
    );
    for (i, (a, b)) in streamed.iter().zip(&one_shot).enumerate() {
        assert!(
            (a - b).abs() < 1e-4,
            "sample {i}: streamed {a} != one-shot {b}"
        );
    }
}

#[test]
fn stereo_keeps_interleaving_and_pitch() {
    // Interleaved stereo: left 440 Hz, right 660 Hz.
    let n = SR as usize * 2;
    let mut input = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / SR as f32;
        input.push((2.0 * std::f32::consts::PI * 440.0 * t).sin());
        input.push((2.0 * std::f32::consts::PI * 660.0 * t).sin());
    }
    let out = stretch(&input, SR, 2, 1.5).unwrap();
    assert_eq!(out.len() % 2, 0, "interleaving broken");

    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
    assert!((dominant_freq(&left) - 440.0).abs() / 440.0 < 0.06);
    assert!((dominant_freq(&right) - 660.0).abs() / 660.0 < 0.06);
}

#[test]
fn reset_clears_state_and_resumes_like_new() {
    let a = sine(300.0, 0.5);
    let b = sine(300.0, 0.5);

    // Feed some audio, reset, then process `b` - result must equal processing
    // `b` on a fresh stretcher (reset leaves no residue from `a`).
    let mut ts = TimeStretch::new(SR, 1).unwrap();
    ts.set_tempo(1.5);
    ts.push(&a);
    let _ = ts.pull(usize::MAX);
    ts.reset();
    ts.push(&b);
    let mut after_reset = ts.pull(usize::MAX);
    after_reset.extend(ts.flush());

    let fresh = stretch(&b, SR, 1, 1.5).unwrap();
    assert_eq!(after_reset.len(), fresh.len());
    for (x, y) in after_reset.iter().zip(&fresh) {
        assert!((x - y).abs() < 1e-4);
    }
    // Tempo survives the reset.
    assert_eq!(ts.tempo(), 1.5);
}

#[test]
fn tempo_change_mid_stream_is_smooth() {
    let input = sine(400.0, 2.0);
    let mut ts = TimeStretch::with_config(SR, 1, Config::default()).unwrap();
    let mut out = Vec::new();
    ts.push(&input);
    // First half at 1.0, then switch to 1.5.
    out.extend(ts.pull(input.len() / 3));
    ts.set_tempo(1.5);
    out.extend(ts.pull(usize::MAX));
    out.extend(ts.flush());
    assert!(out.iter().all(|s| s.is_finite()));
    // Pitch still 400 Hz despite the tempo change.
    assert!((dominant_freq(&out) - 400.0).abs() / 400.0 < 0.06);
}

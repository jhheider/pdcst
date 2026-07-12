//! Property-based coverage: over random audio and tempos, the stretcher must
//! never produce non-finite or runaway samples, must keep length roughly in
//! step with tempo, and must give the same result streamed as in one shot.

use proptest::prelude::*;
use wsola::{TimeStretch, stretch};

const SR: u32 = 44_100;

proptest! {
    // Each case runs the O(n) algorithm over up to ~0.2 s of audio; keep the
    // case count modest so the suite stays quick.
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn never_nonfinite_or_runaway(
        samples in proptest::collection::vec(-1.0f32..=1.0f32, 0..8000),
        tempo in 0.25f32..=4.0f32,
    ) {
        let out = stretch(&samples, SR, 1, tempo).unwrap();
        prop_assert!(out.iter().all(|s| s.is_finite()));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let out_peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        // Overlap-add of Hann-windowed similar segments cannot exceed the peak
        // by much; allow generous slack for correlation mismatches.
        prop_assert!(out_peak <= peak * 2.0 + 1e-3, "peak {peak} -> {out_peak}");
    }

    #[test]
    fn length_tracks_tempo(
        len in 4000usize..16000,
        tempo in 0.5f32..=2.0f32,
    ) {
        let samples = vec![0.05f32; len];
        let out = stretch(&samples, SR, 1, tempo).unwrap();
        let expected = len as f32 / tempo;
        // Loose bound: proportional to 1/tempo, with a few frames of edge slack.
        prop_assert!((out.len() as f32) <= expected * 1.3 + 4000.0);
        prop_assert!((out.len() as f32) + 4000.0 >= expected * 0.7);
    }

    #[test]
    fn streaming_matches_one_shot(
        samples in proptest::collection::vec(-1.0f32..=1.0f32, 0..6000),
        tempo in 0.5f32..=2.0f32,
        chunk in 1usize..2000,
    ) {
        let one_shot = stretch(&samples, SR, 1, tempo).unwrap();

        let mut ts = TimeStretch::new(SR, 1).unwrap();
        ts.set_tempo(tempo);
        let mut streamed = Vec::new();
        for part in samples.chunks(chunk) {
            ts.push(part);
            streamed.extend(ts.pull(256));
        }
        streamed.extend(ts.flush());

        prop_assert_eq!(streamed.len(), one_shot.len());
        for (a, b) in streamed.iter().zip(&one_shot) {
            prop_assert!((a - b).abs() < 1e-4);
        }
    }
}

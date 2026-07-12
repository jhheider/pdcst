//! Pitch-preserved audio time-stretch (WSOLA), pure Rust.
//!
//! Change the tempo of an audio stream without changing its pitch - speed a
//! podcast to 1.5x without the chipmunk effect. Time-domain WSOLA (Waveform
//! Similarity Overlap-Add): good on speech, real-time capable, and far simpler
//! than a phase vocoder. See the design brief (jhheider/briefs
//! `pure-rust-time-stretch`).
//!
//! The API is a streaming push/pull block interface so it drops into a live
//! pipeline between a decoder and an output sink:
//!
//! ```
//! use wsola::TimeStretch;
//! let mut ts = TimeStretch::new(44_100, 2);
//! ts.set_tempo(1.5); // 1.5x faster, same pitch
//! ts.push(&[0.0f32; 1024]); // feed decoded interleaved frames
//! let out = ts.pull(512); // pull stretched frames for the sink
//! # let _ = out;
//! ```
//!
//! NOTE: the WSOLA core is not implemented yet - this is the crate skeleton
//! with the settled public shape. Until the algorithm lands, the stretcher is
//! an identity passthrough (it ignores tempo), so a consumer can wire the
//! pipeline now and get correct 1.0x audio.

/// A streaming, pitch-preserving time-stretcher over interleaved `f32` frames.
pub struct TimeStretch {
    sample_rate: u32,
    channels: u16,
    tempo: f32,
    /// Interleaved samples fed in, not yet consumed by `pull`.
    buffer: Vec<f32>,
}

impl TimeStretch {
    /// Create a stretcher for the given sample rate and channel count.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            tempo: 1.0,
            buffer: Vec::new(),
        }
    }

    /// Playback tempo multiplier: 1.0 is unchanged, 1.5 is 50% faster, 0.5 is
    /// half speed. Pitch is preserved. Takes effect on subsequent output.
    /// Clamped to a sane range; values <= 0 are ignored.
    pub fn set_tempo(&mut self, tempo: f32) {
        if tempo > 0.0 {
            self.tempo = tempo.clamp(0.25, 4.0);
        }
    }

    /// Current tempo multiplier.
    pub fn tempo(&self) -> f32 {
        self.tempo
    }

    /// Sample rate this stretcher was built for.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count (interleaving factor) this stretcher was built for.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Feed interleaved input frames.
    pub fn push(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);
    }

    /// Pull up to `max_samples` interleaved output samples. Returns fewer (or
    /// none) if not enough input has been buffered yet.
    ///
    /// TODO: real WSOLA. Today this is an identity passthrough - it drains the
    /// input buffer unchanged and ignores `tempo`, which is correct only at
    /// 1.0x. Wiring a pipeline against this is safe; speed is a follow-up.
    pub fn pull(&mut self, max_samples: usize) -> Vec<f32> {
        let n = max_samples.min(self.buffer.len());
        self.buffer.drain(..n).collect()
    }

    /// Number of buffered input samples not yet pulled.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passthrough_at_1x() {
        let mut ts = TimeStretch::new(48_000, 2);
        assert_eq!(ts.tempo(), 1.0);
        ts.push(&[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(ts.buffered(), 4);
        let out = ts.pull(4);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(ts.buffered(), 0);
    }

    #[test]
    fn set_tempo_clamps_and_ignores_nonpositive() {
        let mut ts = TimeStretch::new(44_100, 1);
        ts.set_tempo(1.5);
        assert_eq!(ts.tempo(), 1.5);
        ts.set_tempo(99.0);
        assert_eq!(ts.tempo(), 4.0);
        ts.set_tempo(-1.0);
        assert_eq!(ts.tempo(), 4.0); // unchanged
    }

    #[test]
    fn pull_returns_available_when_short() {
        let mut ts = TimeStretch::new(44_100, 1);
        ts.push(&[1.0, 2.0]);
        let out = ts.pull(10);
        assert_eq!(out, vec![1.0, 2.0]);
    }
}

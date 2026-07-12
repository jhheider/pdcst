//! A rodio [`Source`] that time-stretches its inner source with [`wsola`], for
//! pitch-corrected playback speed (a podcast at 1.5x, no chipmunk voices).
//!
//! Two things make it fit the player:
//! - **Tempo is read lock-free** from a shared atomic each refill, so speed
//!   changes from the async side take effect live without touching the audio
//!   callback with a lock.
//! - **Position is tracked in source time** (input samples consumed), not the
//!   stretched output time, and written to a shared atomic - so "12:34 of 45:00"
//!   means the same thing at any speed. Seeking resets the stretch state and the
//!   position to the requested source instant.

use rodio::source::SeekError;
use rodio::{ChannelCount, SampleRate, Source};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use wsola::TimeStretch;

/// Input pulled from the inner source per refill (per channel).
const BLOCK_FRAMES: usize = 2048;

/// Wraps a rodio source and stretches it to `tempo` while preserving pitch.
pub struct WsolaSource<S> {
    inner: S,
    channels: ChannelCount,
    sample_rate: SampleRate,
    ts: TimeStretch,
    /// Tempo multiplier as `f32` bits; read each refill.
    tempo: Arc<AtomicU32>,
    /// Source-time playback position in milliseconds; written each refill.
    position_ms: Arc<AtomicU64>,
    /// Per-channel input frames consumed so far.
    in_frames: u64,
    out: VecDeque<f32>,
    done: bool,
    last_tempo: f32,
}

impl<S: Source> WsolaSource<S> {
    /// Wrap `inner`, driving tempo from `tempo` and reporting source position to
    /// `position_ms`. Both are shared with the player's audio state.
    pub fn new(inner: S, tempo: Arc<AtomicU32>, position_ms: Arc<AtomicU64>) -> Self {
        let channels = inner.channels();
        let sample_rate = inner.sample_rate();
        let mut ts = TimeStretch::new(sample_rate.get(), channels.get())
            .expect("decoder reports nonzero sample rate and channels");
        let last_tempo = f32::from_bits(tempo.load(Ordering::Relaxed));
        ts.set_tempo(last_tempo);
        Self {
            inner,
            channels,
            sample_rate,
            ts,
            tempo,
            position_ms,
            in_frames: 0,
            out: VecDeque::new(),
            done: false,
            last_tempo,
        }
    }

    fn refill(&mut self) {
        let t = f32::from_bits(self.tempo.load(Ordering::Relaxed));
        if t != self.last_tempo {
            self.ts.set_tempo(t);
            self.last_tempo = t;
        }

        let ch = self.channels.get() as usize;
        let mut buf = Vec::with_capacity(BLOCK_FRAMES * ch);
        for _ in 0..BLOCK_FRAMES * ch {
            match self.inner.next() {
                Some(s) => buf.push(s),
                None => break,
            }
        }

        if buf.is_empty() {
            self.done = true;
            let tail = self.ts.flush();
            self.out.extend(tail);
            return;
        }

        self.in_frames += (buf.len() / ch) as u64;
        self.position_ms.store(
            self.in_frames * 1000 / self.sample_rate.get() as u64,
            Ordering::Relaxed,
        );
        self.ts.push(&buf);
        self.out.extend(self.ts.pull(usize::MAX));
    }
}

impl<S: Source> Iterator for WsolaSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(s) = self.out.pop_front() {
                return Some(s);
            }
            if self.done {
                return None;
            }
            self.refill();
        }
    }
}

impl<S: Source> Source for WsolaSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        // Source-time duration (unchanged by tempo).
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        self.ts.reset();
        self.ts.set_tempo(self.last_tempo);
        self.out.clear();
        self.done = false;
        self.in_frames = (pos.as_secs_f64() * self.sample_rate.get() as f64) as u64;
        self.position_ms
            .store(pos.as_millis() as u64, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    const SR: u32 = 44_100;

    /// A finite rodio source over raw samples (helper for the NonZero args).
    fn buffer(channels: u16, data: Vec<f32>) -> SamplesBuffer {
        SamplesBuffer::new(
            NonZero::new(channels).unwrap(),
            NonZero::new(SR).unwrap(),
            data,
        )
    }

    fn tempo_handle(t: f32) -> Arc<AtomicU32> {
        Arc::new(AtomicU32::new(t.to_bits()))
    }

    fn sine(freq: f32, secs: f32) -> Vec<f32> {
        let n = (SR as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin())
            .collect()
    }

    fn dominant_freq(sig: &[f32]) -> f32 {
        let a = sig.len() / 8;
        let b = sig.len() - sig.len() / 8;
        let seg = &sig[a..b];
        let crossings = seg
            .windows(2)
            .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
            .count();
        crossings as f32 / 2.0 / (seg.len() as f32 / SR as f32)
    }

    #[test]
    fn stretches_duration_and_keeps_pitch() {
        let input = sine(440.0, 2.0);
        let inner = buffer(1, input.clone());
        let pos = Arc::new(AtomicU64::new(0));
        let src = WsolaSource::new(inner, tempo_handle(1.5), pos.clone());

        let out: Vec<f32> = src.collect();
        // ~1.5x faster -> ~2/3 the samples.
        let ratio = out.len() as f32 / (input.len() as f32 / 1.5);
        assert!((0.9..=1.1).contains(&ratio), "length ratio {ratio}");
        // Pitch unchanged.
        assert!((dominant_freq(&out) - 440.0).abs() / 440.0 < 0.06);
        // Source-time position advanced to ~the full 2 seconds.
        assert!(pos.load(Ordering::Relaxed) >= 1900);
    }

    #[test]
    fn seek_resets_and_repositions() {
        let inner = buffer(1, sine(330.0, 3.0));
        let pos = Arc::new(AtomicU64::new(0));
        let mut src = WsolaSource::new(inner, tempo_handle(1.0), pos.clone());
        // Consume a little.
        for _ in 0..10_000 {
            src.next();
        }
        src.try_seek(Duration::from_secs(1)).unwrap();
        assert_eq!(pos.load(Ordering::Relaxed), 1000);
        // Still produces audio after the seek.
        assert!(src.next().is_some());
    }

    #[test]
    fn declares_inner_format() {
        let inner = buffer(2, sine(200.0, 0.2));
        let src = WsolaSource::new(inner, tempo_handle(1.0), Arc::new(AtomicU64::new(0)));
        assert_eq!(src.channels().get(), 2);
        assert_eq!(src.sample_rate().get(), SR);
    }
}

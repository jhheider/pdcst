//! Pitch-preserved audio time-stretch (WSOLA), pure Rust, no C.
//!
//! Change the *tempo* of an audio stream without changing its *pitch*: speed a
//! podcast to 1.5x without the chipmunk effect. The engine is WSOLA (Waveform
//! Similarity Overlap-Add): a time-domain method that is real-time capable, good
//! on speech, and far simpler than a phase vocoder.
//!
//! # Why WSOLA (and how it works)
//!
//! Naive tempo change resamples: playing samples faster shifts pitch up. WSOLA
//! instead cuts the input into overlapping frames, lays them back down at a
//! *different* spacing (closer together to speed up, further apart to slow
//! down), and overlap-adds them with a Hann window. Placing frames at a new
//! spacing changes duration; keeping each frame's samples untouched preserves
//! pitch. The "WS" part is the trick that avoids clicks: before laying down each
//! frame, it searches a small window of nearby input positions for the segment
//! whose waveform best *continues* the previous frame, so the overlap-add joins
//! coherent waveforms instead of fighting phases.
//!
//! With a periodic Hann window at 50% overlap the windows sum to unity, so at
//! tempo `1.0` (and a zero-offset match) the output reconstructs the input.
//!
//! # Streaming API
//!
//! [`TimeStretch`] is a streaming processor over interleaved `f32` frames. Feed
//! decoder output with [`push`](TimeStretch::push), pull stretched frames for
//! the sink with [`pull`](TimeStretch::pull), and [`set_draining`](TimeStretch::set_draining)
//! at end of stream. Tempo is settable mid-stream.
//!
//! ```
//! use std::num::{NonZeroU16, NonZeroU32};
//! use wsola::TimeStretch;
//! let sr = NonZeroU32::new(44_100).unwrap();
//! let ch = NonZeroU16::new(2).unwrap();
//! let mut ts = TimeStretch::new(sr, ch).unwrap();
//! ts.set_tempo(1.5); // 1.5x faster, same pitch
//! ts.push(&[0.0f32; 4096]); // interleaved stereo frames from the decoder
//! let mut tmp = vec![0.0f32; ts.min_output_buff_size()];
//! let written = ts.pull(&mut tmp); // interleaved frames for the output sink
//!
//! // flush the final samples
//! ts.set_draining();
//! let written = ts.pull(&mut tmp);
//! # let _ = written;
//! ```
//!
//! For a whole buffer at once, use the [`stretch`] convenience function.

use std::f32::consts::PI;
use std::num::{NonZeroU16, NonZeroU32};

/// Errors from constructing or driving a [`TimeStretch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// An interleaved buffer length was not a whole number of frames.
    #[error("interleaved sample count {len} is not a multiple of {channels} channels")]
    UnalignedInput {
        /// The offending buffer length.
        len: usize,
        /// The configured channel count.
        channels: NonZeroU16,
    },

    /// The given buffer was not at least the necessary size, so nothing could be processed.
    #[error("Given buffer was {given}, but minimal required is {necessary}")]
    NotEnoughBuffer { given: usize, necessary: usize },

    /// The given buffer is not aligned to `necessary`. This is necessary for optimization.
    #[error(
        "Given buffer was {given}, but was not aligned. Next alignment point would be at {next_align}"
    )]
    BufferNotAligned { given: usize, next_align: usize },

    #[error("Nothing could be written as there is not enough input")]
    NotEnoughInput,
}

/// Convenience alias for results from this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Lowest and highest tempo the stretcher will honor; [`TimeStretch::set_tempo`]
/// clamps to this range.
pub const MIN_TEMPO: f32 = 0.25;
/// See [`MIN_TEMPO`].
pub const MAX_TEMPO: f32 = 4.0;

/// Tuning for the WSOLA windows, in milliseconds. [`Config::default`] is a good
/// starting point for speech; larger `hop_ms` is smoother but smears transients,
/// larger `search_ms` improves waveform matching at the cost of CPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Synthesis hop (half the Hann frame) in milliseconds.
    pub hop_ms: f32,
    /// Similarity-search half-range in milliseconds.
    pub search_ms: f32,
}

impl Default for Config {
    fn default() -> Self {
        // ~15 ms hop -> ~30 ms Hann frame; ~12 ms search. Good for speech.
        Self {
            hop_ms: 15.0,
            search_ms: 12.0,
        }
    }
}

/// Custom Epsilon, as [`f32::EPSILON`] may in some cases be too small, when the samples are near 0.0.
const EPSILON: f32 = 1e-9;

/// A streaming, pitch-preserving time-stretcher over interleaved `f32` frames.
///
/// See the [crate docs](crate) for the algorithm and a usage example.
#[derive(Debug, Clone)]
pub struct TimeStretch {
    sample_rate: NonZeroU32,
    channels: NonZeroU16,
    tempo: f32,

    // Geometry, per channel (samples).
    hop: usize,    // Ss: synthesis hop, and the overlap length
    frame: usize,  // 2 * hop: Hann frame length
    search: usize, // similarity-search half-range
    window: Box<[f32]>,

    // Input ring: `input` holds interleaved samples for per-channel absolute
    // positions [origin, origin + input.len()/channels).
    input: Vec<f32>,
    origin: usize,

    // WSOLA state.
    primed: bool,
    // End-of-stream: allow a step to run with a truncated search range.
    // Otherwise a step waits for the full window, so streamed output is
    // bit-identical to processing the whole buffer at once.
    draining: bool,
    ideal: f64,        // next ideal source position (per-channel absolute)
    last_src: usize,   // source of the previously placed frame
    accum: Box<[f32]>, // open output tail (interleaved, hop * channels)
}

impl TimeStretch {
    /// Create a stretcher for `sample_rate` Hz and `channels` interleaved
    /// channels, with the default [`Config`].
    ///
    /// # Errors
    /// Nothing currently; the `Result` is kept for future construction errors.
    pub fn new(sample_rate: NonZeroU32, channels: NonZeroU16) -> Result<Self> {
        Self::with_config(sample_rate, channels, Config::default())
    }

    /// Create a stretcher with explicit [`Config`] tuning.
    ///
    /// # Errors
    /// As [`new`](Self::new).
    pub fn with_config(
        sample_rate: NonZeroU32,
        channels: NonZeroU16,
        config: Config,
    ) -> Result<Self> {
        let sample_rate_f32 = sample_rate.get() as f32;
        let hop = ((sample_rate_f32 * config.hop_ms / 1000.0).round() as usize).max(1);
        let frame = hop * 2;
        let search = ((sample_rate_f32 * config.search_ms / 1000.0).round() as usize).max(1);
        let window = hann(frame).into_boxed_slice();
        Ok(Self {
            sample_rate,
            channels,
            tempo: 1.0,
            hop,
            frame,
            search,
            window,
            input: Vec::new(),
            origin: 0,
            primed: false,
            draining: false,
            ideal: 0.0,
            last_src: 0,
            accum: vec![0.0; hop * channels.get() as usize].into_boxed_slice(),
        })
    }

    /// Playback tempo multiplier: `1.0` unchanged, `1.5` is 50% faster, `0.5`
    /// half speed. Pitch is preserved. Clamped to `[MIN_TEMPO, MAX_TEMPO]`;
    /// non-finite or non-positive values are ignored.
    pub fn set_tempo(&mut self, tempo: f32) {
        if tempo.is_finite() && tempo > 0.0 {
            self.tempo = tempo.clamp(MIN_TEMPO, MAX_TEMPO);
        }
    }

    /// Current (clamped) tempo multiplier.
    pub fn tempo(&self) -> f32 {
        self.tempo
    }

    /// Sample rate this stretcher was built for.
    pub fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    /// Channel count (interleaving factor).
    pub fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    /// Feed interleaved input frames. The caller must keep the buffer
    /// frame-aligned (a whole number of channel groups) across calls.
    pub fn push(&mut self, samples: &[f32]) {
        self.input.extend_from_slice(samples);
    }

    /// Per-channel input samples buffered but not yet consumed.
    pub fn buffered(&self) -> usize {
        self.input.len() / self.channels_usize()
    }

    /// Process samples into `out` buffer.
    /// The buffer needs to be at least [`min_output_buff_size`](Self::min_output_buff_size) and must be a multiple of it.
    ///
    /// If [`set_draining`](Self::set_draining) was called, this will also flush out the final data, if it fits within the buffer.
    /// After a successful drain, the instance can be immediately reused as if newly created.
    ///
    /// Returns the number of bytes written to the output buffer, which should always be a multiple of [`min_output_buff_size`](Self::min_output_buff_size).
    ///
    /// # Errors
    ///
    /// - Buffer is too small for even one run
    /// - Buffer is not a multiple of [`min_output_buff_size`](Self::min_output_buff_size)
    /// - There is not enough input
    pub fn pull(&mut self, out: &mut [f32]) -> Result<usize> {
        let mut total_written = 0;
        let necessary = self.min_output_buff_size();
        let out_len = out.len();

        if out_len < necessary {
            return Err(Error::NotEnoughBuffer {
                necessary,
                given: out_len,
            });
        } else if !out_len.is_multiple_of(necessary) {
            return Err(Error::BufferNotAligned {
                given: out_len,
                next_align: out_len.next_multiple_of(necessary),
            });
        }

        loop {
            let written = self.step(&mut out[total_written..total_written + necessary]);
            total_written += written;

            // No more space to write.
            if total_written >= out_len {
                return Ok(total_written);
            }

            // At this point we know that there is no more data "step" can produce,
            // and we also know that the output buffer still has at least 1 "necessary" length,
            // which is enough to write the tail data.
            if written == 0 {
                // Flush out all the remaining tail data if draining.
                if self.draining && self.primed {
                    let ch = self.channels_usize();
                    for i in 0..self.hop {
                        let base_accum_idx = i * ch;
                        for c in 0..ch {
                            out[total_written] = self.accum[base_accum_idx + c];
                            total_written += 1;
                        }
                    }

                    self.reset();
                }
                self.draining = false;

                if total_written == 0 {
                    return Err(Error::NotEnoughInput);
                }

                return Ok(total_written);
            }
        }
    }

    /// Determine if the instance is currently in draining mode.
    pub fn is_draining(&self) -> bool {
        self.draining
    }

    /// Set the instance to be in draining mode.
    ///
    /// WARNING:
    /// Only set into draining mode once you know you have pushed all the data
    /// that will ever be available before calling [`pull`](Self::pull) again!
    pub fn set_draining(&mut self) {
        self.draining = true;
    }

    /// The minimal output that will be generated by a single [`pull`](Self::pull) call if there is not enough available.
    pub fn min_output_buff_size(&self) -> usize {
        self.hop * self.channels_usize()
    }

    /// Discard all buffered input and output and reset the stretch state, as if
    /// freshly constructed, but keep the configured geometry and tempo. Use
    /// this after seeking the upstream source to a new position.
    pub fn reset(&mut self) {
        self.input.clear();
        self.origin = 0;
        self.primed = false;
        self.draining = false;
        self.ideal = 0.0;
        self.last_src = 0;
        self.accum.fill(0.0);
    }

    #[inline]
    fn channels_usize(&self) -> usize {
        self.channels.get() as usize
    }

    #[inline]
    fn avail_end(&self) -> usize {
        self.origin + self.input.len() / self.channels_usize()
    }

    #[inline]
    fn analysis_hop(&self) -> f64 {
        self.hop as f64 * self.tempo as f64
    }

    /// Process exactly one frame, if the input data is enough.
    ///
    /// Returns bytes written to the `out` buffer, which should always match [`min_output_buff_size`](Self::min_output_buff_size).
    /// Or `0` if nothing was written (not enough input data)
    fn step(&mut self, out: &mut [f32]) -> usize {
        // The last written to index into "out".
        // This should be incremented directly after each write,
        // which then will also serve as the written length.
        let mut last_write_idx = 0;

        let ch = self.channels_usize();
        let hop = self.hop;
        let target_len = self.min_output_buff_size();
        let frame = self.frame;
        let avail_end = self.avail_end();

        let origin = self.origin;
        let input = &self.input;

        if !self.primed {
            if origin + frame > avail_end {
                return 0;
            }

            // Pre-slice the input so that bound checks are done once
            // and have to have less calculations in the loops.
            let input_start_idx = origin * ch;
            let input_slice = &input[input_start_idx..input_start_idx + target_len];
            let window_slice = &self.window[..hop];

            for (i, win) in window_slice.iter().enumerate() {
                let base_idx = i * ch;
                for c in 0..ch {
                    let sample_idx = base_idx + c;
                    out[last_write_idx] = input_slice[sample_idx] * win;
                    last_write_idx += 1;
                }
            }

            // Pre-slice the input so that bound checks are done once
            // and have to have less calculations in the loops.
            let input_slice =
                &input[input_start_idx + target_len..input_start_idx + target_len * 2];
            let window_slice = &self.window[hop..hop * 2];

            for (i, win) in window_slice.iter().enumerate() {
                let base_idx = i * ch;
                for c in 0..ch {
                    let sample_idx = base_idx + c;
                    self.accum[sample_idx] = input_slice[sample_idx] * win;
                }
            }
            self.last_src = origin;
            self.ideal = origin as f64 + self.analysis_hop();
            self.primed = true;
            self.compact();

            return last_write_idx;
        }

        let base = self.ideal.round() as i64;
        let cmin = origin as i64;
        let cmax = avail_end as i64 - frame as i64;
        let target = self.last_src + hop;

        // The highest input index this step could read: the top candidate's
        // frame end, or the correlation target's end. Unless we are draining the
        // last frames at end of stream, wait until all of it is buffered, so the
        // search sees the same full candidate range it would in one shot.
        let needed_end = (base + self.search as i64 + frame as i64).max((target + hop) as i64);
        if !self.draining && needed_end > avail_end as i64 {
            return 0;
        }
        // Even draining, we need at least one full candidate frame and the target.
        if cmax < cmin || target + hop > avail_end {
            return 0;
        }

        let lo = (base - self.search as i64).max(cmin);
        let hi = (base + self.search as i64).min(cmax);
        if hi < lo {
            return 0;
        }

        // Pre-slice the input so that bound checks are done once
        // and have to have less calculations in the loops.
        let input_start_idx = (target - origin) * ch;
        let input_slice = &input[input_start_idx..input_start_idx + target_len];

        // Pick the source frame whose leading overlap best correlates with the
        // natural continuation of the previous frame (input at `target`).
        let mut best = lo as usize;
        let mut best_score = f32::NEG_INFINITY;

        // Calculate target energy once
        let target_energy = input_slice.iter().map(|&x| x * x).sum::<f32>().sqrt();

        for cand in lo..=hi {
            let cand = cand as usize;
            let cand_start_idx = (cand - origin) * ch;
            let cand_slice = &input[cand_start_idx..cand_start_idx + target_len];

            let mut dot_product = 0.0f32;
            let mut cand_energy = 0.0f32;

            for i in 0..target_len {
                let c_sample = cand_slice[i];
                let t_sample = input_slice[i];

                dot_product += c_sample * t_sample;
                cand_energy += c_sample * c_sample;
            }

            // Normalized Cross-Correlation (Cosine Similarity)
            // Adds a small Epsilon to avoid divide by zero.
            let score = dot_product / (cand_energy.sqrt() * target_energy + EPSILON);

            if score > best_score {
                best_score = score;
                best = cand;
            }
        }
        let src = best;

        // Pre-slice the input so that bound checks are done once
        // and have less indexing calculations in the loop.
        let input_start_idx = (src - origin) * ch;
        let src_slice_first = &input[input_start_idx..input_start_idx + target_len];
        let src_slice_second =
            &input[input_start_idx + target_len..input_start_idx + target_len * 2];
        let window_slice = &self.window[..hop];

        // Overlap-add the frame's first half onto the open tail, emit it, and
        // keep the windowed second half as the new tail.
        for (i, win) in window_slice.iter().enumerate() {
            let base_idx = i * ch;
            for c in 0..ch {
                let sample_idx = base_idx + c;
                let v = self.accum[sample_idx] + src_slice_first[sample_idx] * win;
                out[last_write_idx] = v;
                last_write_idx += 1;
            }
        }

        let window_slice = &self.window[hop..hop * 2];

        for (i, win) in window_slice.iter().enumerate() {
            let base_idx = i * ch;
            for c in 0..ch {
                let sample_idx = base_idx + c;
                self.accum[sample_idx] = src_slice_second[sample_idx] * win;
            }
        }

        self.last_src = src;
        self.ideal += self.analysis_hop();
        self.compact();

        last_write_idx
    }

    /// Drop input we can never read again, in batches to keep it amortized O(n).
    fn compact(&mut self) {
        let next_cand_lo = (self.ideal.round() as i64 - self.search as i64).max(0) as usize;
        let keep_from = self.last_src.min(next_cand_lo);
        if keep_from > self.origin + (1 << 16) {
            let drop = (keep_from - self.origin) * self.channels_usize();
            self.input.drain(0..drop);
            self.origin = keep_from;
        }
    }
}

/// A periodic Hann window of length `n` (`w[i] = 0.5 - 0.5*cos(2*pi*i/n)`), which
/// sums to unity at 50% overlap so overlap-add reconstructs gain exactly.
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

/// Time-stretch a whole interleaved buffer in one call: pitch-preserved tempo
/// change by `tempo`. Equivalent to `new` + `set_tempo` + `push` + `pull` +
/// `set_draining`, and a good first port of call for offline use and tests.
///
/// # Errors
/// [`Error::UnalignedInput`] if `samples` is not a whole number of frames.
///
/// ```
/// # use std::num::{NonZeroU16, NonZeroU32};
/// let out = wsola::stretch(
///     &[0.0f32; 8000],
///     NonZeroU32::new(8000).unwrap(),
///     NonZeroU16::new(1).unwrap(),
///     2.0,
/// )
/// .unwrap();
/// assert!(out.len() < 8000); // roughly half as long at 2x
/// ```
pub fn stretch(
    samples: &[f32],
    sample_rate: NonZeroU32,
    channels: NonZeroU16,
    tempo: f32,
) -> Result<Vec<f32>> {
    if !samples.len().is_multiple_of(channels.get() as usize) {
        return Err(Error::UnalignedInput {
            len: samples.len(),
            channels,
        });
    }
    let mut ts = TimeStretch::new(sample_rate, channels)?;
    ts.set_tempo(tempo);
    ts.push(samples);
    ts.set_draining();

    let mut out = Vec::new();
    let mut tmp = vec![0.0f32; ts.min_output_buff_size() * 4].into_boxed_slice();

    while let Ok(written) = ts.pull(&mut tmp)
        && written > 0
    {
        out.extend_from_slice(&tmp[..written]);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_config() {
        assert_eq!(
            stretch(
                &[0.0; 3],
                NonZeroU32::new(8000).unwrap(),
                NonZeroU16::new(2).unwrap(),
                1.0
            )
            .unwrap_err(),
            Error::UnalignedInput {
                len: 3,
                channels: NonZeroU16::new(2).unwrap()
            }
        );
    }

    #[test]
    fn set_tempo_clamps_and_ignores_bad() {
        let mut ts = TimeStretch::new(
            NonZeroU32::new(44_100).unwrap(),
            NonZeroU16::new(1).unwrap(),
        )
        .unwrap();
        assert_eq!(ts.tempo(), 1.0);
        ts.set_tempo(1.5);
        assert_eq!(ts.tempo(), 1.5);
        ts.set_tempo(100.0);
        assert_eq!(ts.tempo(), MAX_TEMPO);
        ts.set_tempo(0.01);
        assert_eq!(ts.tempo(), MIN_TEMPO);
        ts.set_tempo(-1.0);
        assert_eq!(ts.tempo(), MIN_TEMPO); // unchanged
        ts.set_tempo(f32::NAN);
        assert_eq!(ts.tempo(), MIN_TEMPO); // unchanged
    }

    #[test]
    fn empty_and_tiny_inputs_do_not_panic() {
        assert!(
            stretch(
                &[],
                NonZeroU32::new(44_100).unwrap(),
                NonZeroU16::new(1).unwrap(),
                1.5
            )
            .unwrap()
            .is_empty()
        );
        // Shorter than a frame: no full frame to place; must not panic.
        let out = stretch(
            &[0.1, -0.1, 0.2],
            NonZeroU32::new(44_100).unwrap(),
            NonZeroU16::new(1).unwrap(),
            2.0,
        )
        .unwrap();
        assert!(out.iter().all(|s| s.is_finite()));
    }
}

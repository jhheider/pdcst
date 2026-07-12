# wsola

Pitch-preserved audio time-stretch (WSOLA), pure Rust, no C.

Change the *tempo* of an audio stream without changing its *pitch* - speed a
podcast to 1.5x without the chipmunk effect. The engine is WSOLA (Waveform
Similarity Overlap-Add): a time-domain method that is real-time capable, good on
speech, and far simpler than a phase vocoder.

- Pure Rust, one small dependency (`thiserror`), no C toolchain or `-sys` crates.
- Streaming *and* one-shot APIs over interleaved `f32` frames.
- Live tempo changes (set the tempo mid-stream).
- Property-tested; streaming output is bit-identical to one-shot.

## Usage

Streaming (feed decoder output, pull frames for the sink):

```rust
use wsola::TimeStretch;

let mut ts = TimeStretch::new(44_100, 2).unwrap(); // sample rate, channels
ts.set_tempo(1.5);                                 // 1.5x faster, same pitch
ts.push(&[0.0f32; 4096]);                          // interleaved stereo frames
let out = ts.pull(1024);                           // interleaved frames out
// ... at end of stream:
let tail = ts.flush();
```

One-shot (stretch a whole buffer):

```rust
use wsola::stretch;

let input = vec![0.0f32; 44_100 * 2]; // 1s of stereo
let out = stretch(&input, 44_100, 2, 1.5);
```

Tempo is a multiplier: `> 1.0` is faster/shorter, `< 1.0` is slower/longer, and
`1.0` reconstructs the input. Interleaved `f32` in, interleaved `f32` out.

## How it works

Naive tempo change resamples, which shifts pitch. WSOLA instead cuts the input
into overlapping frames, lays them back down at a *different* spacing (closer to
speed up, further apart to slow down), and overlap-adds them with a Hann window.
New spacing changes duration; leaving each frame's samples untouched preserves
pitch. Before laying down each frame it searches a small window of nearby input
positions for the segment whose waveform best *continues* the previous frame, so
the overlap-add joins coherent waveforms instead of fighting phases - that is the
"waveform similarity" that avoids clicks.

See the crate docs for the full API and the algorithm notes.

## License

MIT.

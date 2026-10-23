# Native audio engine

All processing uses 48 kHz float PCM in 480-sample (10 ms) frames. Voice is mono
64 kbps VBR Opus/VOIP with FEC and DTX; Studio is stereo 192 kbps Opus/AUDIO with
APM and denoising bypassed. Packets are 20 ms by default, or 10 ms on request.

## Using it

`SendPipeline::process` accepts a `StereoFrame` (`process_mono` is also available)
and returns an Opus packet every one or two calls. `render_reference` accepts the
actual speaker mix for echo cancellation. `ReceivePipeline` accepts packets with
monotonic arrival milliseconds and emits one stereo frame per 10 ms audio tick.
Keep calling `render` during loss: it schedules FEC/PLC rather than leaving a hole.

`set_settings` changes the profile, packet duration, noise suppression, AGC, gate,
volumes, pan, normalization and deafen state at runtime. Codec changes preserve the
RTP clock and discard a partial old packet. `Gate` supports Open, VoiceActivation,
PushToTalk and Muted. Studio's default voice-activation gate stays open for music.

`DeviceIo` owns CPAL streams and SPSC rings. Call capture/playback and DSP from a
worker; callbacks only convert samples, push/pop fixed stereo sample frames, and
update atomics. Rings hold at most 100 ms. Resampling uses rubato's windowed sinc
outside callbacks, with aligned stereo channels. Devices are selected by listed
name (duplicate names are currently ambiguous). A missing selected device falls
back to the default. Call `recover` after a reported stream error; it retries at
most once per second and re-enumerates devices. A failed switch keeps old streams.

`rtc::Session` is the sans-I/O signaling/WebRTC state machine. Drain its
`poll_output` to Timeout after every mutation, forwarding Transmit datagrams and
its queued signaling messages. `rtc::Client` supplies tokio UDP and WebSocket I/O,
a command channel and broadcast events (subscribers must handle `Lagged`). Run
`Client::run` on a dedicated worker/runtime thread when embedding in Tauri. Its
Settings command also switches devices and publishes mute/deafen state. With no
DeviceIo, Capture commands supply synthetic/external audio. Participant volume
settings use participant IDs; internally mic and screen audio have separate
playout buffers. No durable or high-frequency state is written to a database.

```sh
cargo run -p pingo-media --example devices
cargo run -p pingo-media --release --example loopback -- --loss 5 --jitter 30
cargo run -p pingo-media --release --example join -- --url ws://localhost:3000/rtc --token TOKEN
```

Loopback also accepts `--seconds`, `--input`, `--output`, `--studio` and
`--low-latency`. Join accepts `--input`/`--output`. Device examples need hardware;
none of the regular tests do.

## Adaptive playout

The buffer extends wrapping RTP sequence numbers/timestamps, reorders and
deduplicates packets, and measures the 95th-percentile transit spread over the
last 200 unique arrivals, **including late packets**. Target delay is 20–200 ms,
with fast growth and slow decay. Quiet frames absorb up to 10% delay correction;
speech normally uses 1%, increasing to 5% for large steps. A raised-cosine
splice/overlap-add preserves the channel alignment. This is a first overlap-add
controller, not full pitch-search WSOLA or Google's NetEQ implementation.

A missing packet uses the next packet's Opus FEC without consuming that packet;
bursts without lookahead use PLC. RTP timestamps support 10/20 ms changes at a
loss. Corrupt compressed payloads fall back to PLC. Suspended workers re-anchor
expired audio within the hard delay bound. Storage is bounded at 64 packets.

Stats expose current/target delay, jitter, cumulative loss/concealment/late
percentages, duplicate counts, FEC attempts, PLC, corrupt packets, time correction
and resets. libopus can substitute PLC when LBRR is absent: **FEC attempts are not
claimed as verified recoveries**. Concealment includes all missing/corrupt slots,
including FEC attempts. It is a packet percentage rather than a sample percentage.

## Optional processors and current limits

- Default builds use a pure Rust 80 Hz high-pass and gated basic AGC, **without
  acoustic echo cancellation**. `SendPipeline::has_aec()` reports actual support.
- `--features webrtc-apm` uses `webrtc-audio-processing 2.1` with its bundled
  feature: AEC3, AGC2 and high-pass, with the playback reference fed to AEC. The
  bundled build was attempted on this host and failed with `Failed to execute
  meson` / `No such file or directory`. Thus it is not a default feature here and
  its native build/runtime remain unverified. The upstream build also requires
  Ninja, a C++ toolchain, libclang, Abseil (or its build-time fetch) and Rust's
  llvm-tools component for symbol prefixing.
- RNNoise (`nnnoiseless`) is the working default suppressor, with Off/Gentle/Strong
  levels. Gentle mixes 50% dry with wet; the dry signal is delayed 10 ms to align
  with RNNoise and avoid comb filtering. Strong uses the wet signal only. Weights
  ship with the crate; no runtime downloads occur.
- `--features deepfilter` **builds**, but is an explicit unavailable-backend stub.
  Published `deep_filter 0.2.5` (library name `df`) contains FFT/ERB processing,
  without an inference runner or bundled model. Its DSP state is instantiated to
  verify compatibility; `denoise::deepfilter()` returns an explanatory error.
  Shipping an offline model and inference integration is future work.
- Loudness normalization integrates roughly 400 ms of gated energy with slow
  gain changes. It is not a certified EBU R128 meter (no K-weighting/true-peak
  meter). Spatial output uses constant-power panning, not HRTF.
- RTC currently gathers UDP host candidates, uses SDP-only ICE and the existing
  pingo-proto Join/Offer/Answer/PublishTracks contract. It supports server
  re-offers, client-wins glare, RFC 6464, the pingo data channel and track mapping.
  STUN/TURN, ICE-TCP, Opus RED, reconnect and real SFU/browser interoperability
  are subsequent integration work. Only 10/20 ms remote Opus packets are accepted.
- Hardware hot-plug, acoustic listening quality and real microphone-to-speaker
  latency have not been tested. Seeded 5% loss plus bursts is exercised, but this
  does not establish the product's subjective “no audible artifacts” target.

## Verification and performance

```sh
cargo test -p pingo-media
cargo test -p pingo-media --features deepfilter
cargo check -p pingo-media --examples
cargo clippy -p pingo-media --all-targets --no-deps -- -D warnings
cargo fmt -p pingo-media --check
cargo test -p pingo-media --release --test timing -- --ignored --nocapture
```

Validation passes: 18 default tests, 19 with `deepfilter`; the hardware and timing
tests are ignored by default. Examples compile, package formatting passes, and
strict Clippy without dependency linting passes.

Default tests cover synthetic DSP, RNNoise suppression/scaling, 44.1/96 kHz
resampling, Opus profiles/PLC/FEC, runtime controls, mixer gain/pan/limiting,
sequence/timestamp wrap, seeded loss bursts/reordering/duplicates, delay growth
and recovery, ±1500 ppm clock drift for 60 simulated seconds, suspension, corrupt
payloads, and duration changes during loss. The RTC test negotiates with a real
str0m answerer and transports encrypted audio and data entirely in memory,
including a server re-offer and remote-track routing. Hardware and timing tests
are ignored in ordinary test runs.

The plain `cargo clippy -p pingo-media --all-targets` succeeds but reports a
pre-existing `doc_lazy_continuation` warning at `pingo-proto/src/signal.rs:11`.
`--no-deps -- -D warnings` verifies this crate strictly without changing proto.

Measured 2026-10-23 on AMD Ryzen 7 5800X3D, Rust 1.99 release, system libopus 1.6.1,
default fallback APM/Gentle RNNoise/20 ms Voice, one receive participant, 200 warmup
frames followed by 5,000 timed frames:

- Mean **183.96 µs per 10 ms frame**, median 252.22 µs, p95 336.14 µs;
  **1.84% of one core**. DSP, encode, jitter/decode, mix and their allocations are
  included. Device/transport costs are excluded; this is a shared build machine.
- Chirp correlation finds **46.42 ms** between input/output sample arrays
  (correlation 0.973). The simulator records each output frame at its capture
  completion tick: +10 ms places it on the wall clock, and a conservative +10 ms
  playback quantum gives **66.42 ms** send-to-receive algorithmic latency.
- `latency_budget_ms(20.0)` is a conservative **76.50 ms** budget: 10 ms capture,
  20 ms packetization, 10 ms RNNoise, 6.5 ms Opus lookahead, 20 ms playout and
  10 ms playback. It intentionally allows a full packetization interval.
  These values exclude network transit, device/OS buffering and resampler delay;
  the latter is available from `RateConverter::delay_samples()`.

No signaling or shared protocol contracts were changed.

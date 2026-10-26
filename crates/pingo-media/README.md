# Native media engine

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
cargo run -p pingo-media --release --example join -- --url ws://127.0.0.1:8090/rtc --token TOKEN
```

Loopback also accepts `--seconds`, `--input`, `--output`, `--studio` and
`--low-latency`. Join accepts `--input`/`--output`, `--studio`, `--seconds N` and
`--headless` (synthetic 440 Hz capture). Set `PINGO_MEDIA_TOKEN` instead of
`--token` to keep admission tokens out of command-line output. Device examples
need hardware; none of the regular tests do.

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

- Default builds enable `webrtc-apm`: bundled `webrtc-audio-processing 2.1`,
  AEC3 (full echo canceller), adaptive AGC2, split-band high-pass, and **APM noise
  suppression disabled** because DeepFilterNet follows it. Studio bypasses APM.
  The actual stereo receive mix feeds the reverse stream every 10 ms; mono
  capture feeds the forward stream. CPAL capture/playback timestamps, ring
  occupancy and resampler delay supply stream delay before capture processing.
  External I/O can call `SendPipeline::set_stream_delay_ms`; absent a measurement,
  AEC3 estimates delay. Build prerequisites: Meson, Ninja, C++ toolchain, libclang,
  Abseil (or Meson's build-time fetch) and Rust llvm-tools for symbol prefixing.
- `--no-default-features` keeps the pure Rust 80 Hz high-pass and basic AGC,
  without echo cancellation. `SendPipeline::has_aec()` reports actual support.
- Hardware-free `tests/aec.rs` uses 40 ms delayed, attenuated, filtered synthetic
  speech with room reflections. After 12 s convergence it requires ≥15 dB ERLE
  (>96.8% echo-power reduction), and double-talk near-end gain between 0.5 and
  1.5 so silencing the microphone cannot pass. Measured ERLE and gain are printed
  by `cargo test -p pingo-media --test aec -- --nocapture`.
- Default builds enable `deepfilter`: **DeepFilterNet3** (`crates/pingo-denoise`) is
  the suppressor. Gentle caps attenuation at 18 dB; Strong removes as much as the
  model can. Strength changes apply in place. It adds 30 ms of delay (RNNoise: 10 ms)
  and about 4% of one core. The model is compiled in; there are no runtime downloads.
  `denoise::preload()` parses it ahead of the first call (the desktop app does this at startup).
- Without `deepfilter`, or if the model fails to load, RNNoise (`nnnoiseless`) is the
  fallback. Its Gentle mixes 50% dry, delayed 10 ms to align with RNNoise and avoid comb filtering.
- Loudness normalization integrates roughly 400 ms of gated energy with slow
  gain changes. It is not a certified EBU R128 meter (no K-weighting/true-peak
  meter). Spatial output uses constant-power panning, not HRTF.
- RTC currently gathers UDP host candidates, uses SDP-only ICE and the existing
  pingo-proto Join/Offer/Answer/PublishTracks contract. It supports server
  re-offers, client-wins glare, RFC 6464, the pingo data channel and track mapping.
  Native clients are verified against the embedded real SFU, including wildcard
  client/server sockets. STUN/TURN deployment, ICE-TCP, native Opus RED, reconnect
  remain future work; native video/browser interoperability is covered below. Only 10/20 ms remote Opus
  packets are accepted.
- Hardware hot-plug, acoustic listening quality and real microphone-to-speaker
  latency have not been tested. Seeded 5% loss plus bursts is exercised, but this
  does not establish the product's subjective “no audible artifacts” target.

## Verification

```sh
cargo test --workspace
cargo test -p pingo-media --no-default-features
cargo check -p pingo-media --examples
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo test -p pingo-media --test aec -- --nocapture
```

The default media suite has 20 hardware-free tests, plus ignored hardware and
release-timing tests. Coverage includes AEC convergence/double-talk, DC rejection,
DeepFilterNet and RNNoise suppression, resampling, Opus profiles/PLC/FEC, runtime controls, mixing, RTP wrap,
loss bursts/reordering/duplicates, adaptive delay and clock drift, corrupted
payloads, and encrypted in-memory RTC audio/data/re-offer/glare negotiation.
The server adds five native-client integration scenarios described below.

Measured AEC3 ERLE: **25.99 dB**, with **0.757 near-end gain** during double talk.
AGC is disabled for this measurement so gain changes cannot inflate ERLE. The
normal Voice configuration enables adaptive AGC2. The stereo render reference is antiphase, so averaging it to mono would erase
the reference and fail the test. The test uses a synthetic linear acoustic path; nonlinear loudspeakers and real-room echo still need
acoustic listening/measurement.

No shared signaling/data-channel/backend API contracts changed. The lockfile
includes the native media engine as a server test dependency.

## Full-server native integration

`cargo test -p pingo-server native_ -- --nocapture` starts RxFerrix and the
embedded SFU in-process on ephemeral localhost HTTP/UDP ports. Users obtain
admission through authenticated HTTP `voice:join`. Both endpoints use the native
`rtc::Client`, `SendPipeline`, and `ReceivePipeline`; `run_with_playback` exposes
bounded mixed PCM without CPAL. It checks 440 Hz plus speech-like bursts, marker
click latency below the 150 ms product budget, authenticated Speaking messages,
server mute/query state, disconnect cleanup, and independent Studio stereo
channels. Negotiated SDP requires stereo, FEC and a 256 kbps receive ceiling;
Studio's encoder uses 192 kbps VBR.

Measured on localhost: Voice **67.07 ms**, Studio **57.52 ms** from synthetic
capture to mixed playback, steady-state jitter-buffer delay **29.1–29.7 ms**,
zero lost/concealed/late/corrupt packets. These include packetization, codec,
playout and worker scheduling, with a 10 ms capture quantum; they exclude actual
hardware/OS latency. Run output prints each receiver's full jitter statistics.

The audio tests found two interop bugs: SFU SDP inherited str0m's mono
receive default, and client mute left old audio running through PLC. The SFU
now advertises stereo explicitly (and a 256 kbps receive ceiling, which str0m
0.24 cannot represent in FormatParams); the client drops queued audio and
silences the mix on authoritative participant mute updates.

The three-party case joins a third authenticated member and sends independent
440/880 Hz Voice streams. B's mixed output must contain both narrow-band peaks
(and no comparable 600 Hz peak), with two independent playout buffers. Measured
amplitudes were 0.114/0.097, delays 26.3 ms, and zero loss/concealment.

`Command::PublishAudio(false/true)` sends a complete PublishTracks snapshot and
an inactive/send-only offer. Requests during a pending offer are applied after
its answer. Full-server tests cover unpublish, republish onto a new receiving
mid, stale-buffer removal and mute/unmute recovery; the in-memory RTC test
covers deferred changes and offer glare. Playback queue overflow now feeds
silence to AEC instead of a reference that never reached the speaker.


The wildcard-socket regression reproduces the production server binding with a
concrete advertised loopback address. The SFU previously passed `0.0.0.0` to
str0m as the receive destination, causing ICE to discard every binding request.
It now resolves a concrete advertised candidate using the local route (or the
configured public candidate behind NAT), with a bounded per-peer-address cache.

## Join a development server with real devices

From the repository root, start the server. Explicit temporary codegen avoids
writing into the web agent's directory. Localhost ICE needs a loopback candidate:

```sh
PINGO_PUBLIC_IPS=127.0.0.1 PINGO_UDP_PORT=0 \
  cargo run -p pingo-server -- serve --dev --http 127.0.0.1:8090 \
  --dir /tmp/pingo-media-dev --codegen-out /tmp/pingo-media-codegen
```

In another terminal, list devices and build the example before minting the
60-second media admission token:

```sh
cargo run -p pingo-media --example devices
cargo build -p pingo-media --example join
```

Run this Python 3 snippet from the repository root. It calls `dev:seed`, requests
a magic-link email for demo user Alex, retrieves the dev mailer's code, verifies
it over HTTP, finds the Lounge and calls authenticated `voice:join`. The token
stays in the child process environment. No dashboard/admin setup is required.

```python
import json, os, subprocess, urllib.request

base_url = "http://127.0.0.1:8090"
def post(path, data, access=None):
    headers = {"Content-Type": "application/json"}
    if access:
        headers["Authorization"] = "Bearer " + access
    request = urllib.request.Request(base_url + path, json.dumps(data).encode(), headers)
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)

def call(kind, path, args, access=None):
    reply = post("/api/" + kind, {"path": path, "args": args}, access)
    assert reply["status"] == "success", reply
    return reply["value"]

space = call("mutation", "dev:seed", {})
email = "alex@pingo.test"   # Also seeded: sam@pingo.test, maya@pingo.test.
post("/api/auth/magic/request", {"email": email})
code = call("action", "dev:last_code", {"email": email})
access = post("/api/auth/magic/verify", {"email": email, "code": code})["accessToken"]
rooms = call("query", "rooms:list", {"space": space}, access)
room = next(r["_id"] for r in rooms if r["name"] == "Lounge")
admission = call("mutation", "voice:join", {"room": room}, access)
subprocess.run([
    "target/debug/examples/join", "--url", "ws://127.0.0.1:8090/rtc", "--seconds", "10",
    # Optionally append: "--input", "DEVICE", "--output", "DEVICE", "--studio".
    # Append "--headless" to send a synthetic signal without opening devices.
], env={**os.environ, "PINGO_MEDIA_TOKEN": admission["token"]}, check=True)
print("Participants after leave:", call("query", "voice:participants", {"room": room}, access))
```

Remove `--seconds 10` for an ongoing call; Ctrl-C leaves cleanly. Run the snippet
again as Sam in a second terminal to test hearing each other. Use headphones.
`dev:last_code` and `dev:seed` are available only in `--dev`.

Manual device run on 2026-10-23: HTTP seed/magic-link/join succeeded; the example
opened **HyperX SoloCast** input and **HyperX Cloud III Wireless** output, reached
`Connected`, ran for five seconds, and exited 0 with no DeviceError events. RTT
reports ranged **2.4–13.6 ms**; `voice:participants` was empty after leave. ALSA
printed unavailable pulse/jack/oss alias warnings during enumeration; selected
CPAL streams still opened. This confirms device opening, RTC and cleanup. It
does not establish acoustic AEC quality or physical mouth-to-ear latency.


Final verification (2026-10-23): `cargo test --workspace` passed **63 tests**
(two hardware/performance tests ignored); `cargo clippy --workspace --all-targets
-- -D warnings` and `cargo fmt --all --check` passed. Fallback media tests,
`--features deepfilter` tests and `cargo check -p pingo-media --examples` passed.
The headless join example also reached Connected and exited 0 with no remaining
voice states. DeepFilter inference, STUN/TURN/native RED, browser/acoustic
interoperability, hardware hot-plug and physical mouth-to-ear measurements remain
outside this verification.

## Native capture (Part A)

Build with `--features linux-capture` for the ashpd ScreenCast portal and
PipeWire screen/camera/app-audio backends in `video::capture::linux`. Screen
sources are portal picker categories (`monitor`, `window`), not an enumeration
that bypasses permission. Cursor Hidden/Embedded and portal restore tokens are
supported; persist the returned token in the embedding application's settings.
The portal may ignore persistence or revoke access; start errors propagate.
Always await `stop()` to close the portal session immediately.
The example closes the session and stops its blocking workers on RTC errors and
cancellation as well as normal leave. Video capture keeps one replaceable latest
frame; audio retains ordered 10 ms quanta. Startup errors preserve their cause.

Capture workers own their PipeWire loops; video queues are bounded and return
the newest frame. Timestamps use PipeWire header PTS relative to the first frame
(with monotonic receipt time fallback); damage is exposed when supplied.
Only BGRx/RGBx CPU-mapped buffers are negotiated. DMA-BUF import is **not
implemented**, and no modifier format is advertised. There is no GPU zero-copy.
Cameras enumerate PipeWire `Video/Source` nodes by stable object serial. A camera
must offer packed RGB through its PipeWire adapter; native MJPEG/YUYV conversion
is not yet provided. macOS ScreenCaptureKit and Windows.Graphics.Capture remain
explicit platform TODO modules.

`cargo test -p pingo-media --features linux-capture --test capture -- --ignored
--nocapture` opens the real portal picker and checks capture → encode → decode.
It is excluded from normal hardware-free runs. `screenshare --capture-only`
measures both simulcast layers without needing a room; `--synthetic` replaces
portal input for repeatable measurements.

`AppAudioCapturer` captures one application's playback ports as 48 kHz stereo
10 ms PCM frames. Select from `list_sources()`, which excludes this process,
Pingo-named nodes and nodes with unknown ownership. The selected serial is
revalidated at start and reconnection is disabled. `SystemAudioCapturer` mixes
these application monitor ports into one stereo stream, excluding the same
Pingo/unknown-owner nodes. It takes a snapshot at start; restart to include
newly opened application streams. Closed sources are removed, and an empty mix
emits silence on the 10 ms clock. `screenshare --audio-system` selects this mix;
`--audio-app SERIAL` selects one application. A raw hardware sink monitor is
not exposed because that mix would include call playback. Feed these frames to
the RTC screen-audio input, which encodes stereo Opus without microphone DSP.

## Video encoding (Part B)

`video::color::I420Frame` converts BGRx/RGBx (including padded rows) to BT.709
limited-range I420 using yuvutils-rs SIMD, scales each Y/U/V plane with
fast_image_resize SIMD, and interleaves NV12 for hardware upload. Odd capture
sizes crop the final row/column. Text defaults to 15 fps (configurable 5–15),
up to 3840×2160, with OpenH264 screen-content tuning; Motion allows 1920×1080
at 60 fps. These are requested ceilings, not a real-time throughput guarantee.

`VideoEncoder` supports bitrate changes and explicit keyframe requests.
`SimulcastEncoder` uses independent encoders: screen `l,h` at half/full size,
camera `l,m,h` at quarter/half/full size, with actual sizes/bitrates advertised
as RID attributes. Bitrate changes use 20% hysteresis and preserve capture
clocks; reinitializing a codec emits an IDR. Layers remain active; publisher-side
dynacast is not implemented because the signaling contract carries no layer
subscriber counts.

OpenH264 0.9.9 is the default software encoder/decoder: the Rust wrapper is
BSD-2-Clause and the bundled Cisco OpenH264 source is BSD-2-Clause. Cargo's
`source` feature compiles that source locally (no runtime binary download).
Cisco's royalty coverage for its separately distributed binary does **not**
automatically cover this source-built artifact; distributors must assess H.264
patent obligations. This project does not claim royalty-free H.264 distribution.

`--features vp8` enables system libvpx via ffmpeg-next. The published
`vpx-encode 0.6.2` failed to build its generated encoder config bindings against
this machine's libvpx/Clang, so it is not included. libvpx is BSD-3-Clause;
ffmpeg-next is WTFPL, and system FFmpeg has its own build-dependent LGPL/GPL
license. In particular this machine's system FFmpeg enables GPL components;
redistribution must comply with that installed build. No x264 API is used here.

Linux `--features vaapi` enables FFmpeg h264_vaapi with NV12 upload, no B frames,
a small hardware frame pool and runtime OpenH264 fallback on initialization,
encoding or bitrate-reconfiguration failure. `PINGO_VAAPI_DEVICE` selects a
render node (default `/dev/dri/renderD128`). FFmpeg/libva headers and libraries
are required for these optional features. DMA-BUF zero-copy and NVENC are absent.

## Native video RTC API (Part C)

`Command::PublishVideo { source, config: Some(config) }` starts an encoder
worker and publishes screen/camera simulcast; `None` stops/unpublishes it.
Feed `Command::VideoFrame { source, frame }` from your capture worker.
`Command::EncodedVideo` permits an external encoder (its frames must have a
negotiated RID); low-level `Session::set_video_published`/`send_video` also work.
Encoding queues are bounded, capture frames may be dropped under load, and an
encoded-output overflow requests fresh independent keyframes. The RTC worker
services encoded output every 10 ms without encoding on the audio/UDP clock.

`PublishScreenAudio(true)` plus `CaptureScreenAudio` sends an independent
192 kbps stereo Opus/AUDIO stream, with no AEC/AGC/denoiser. Capture it with
`AppAudioCapturer` to exclude call output. The microphone and screen-audio
packet clocks and remote playout buffers are independent.

`RtcEvent::VideoFrame { track, frame }` carries participant/mid/kind, codec,
monotonic media timestamp, keyframe flag and shared encoded bytes (H.264 Annex B
or raw VP8). Received width/height are zero: the decoder obtains dimensions
from SPS/VP8 headers. This is ready to forward to the desktop app's encoded
frame channel; WebCodecs H.264 must accept Annex B or the consumer must convert
it to AVCC. Broadcast consumers must handle `Lagged` by requesting a keyframe
with `Command::RequestVideoKeyframe`. `SetVideoQuality` selects remote height,
including zero to pause. `VideoKeyframeRequested` and `BandwidthEstimate` events
are also exposed for external encoders; built-in workers consume them directly.

The client now uses str0m's frame API for both audio and video, leaving RTP
packetization, retransmission and video depacketization to str0m. Existing Opus
level headers and adaptive playout remain covered by regression tests. Start,
stop and configuration changes serialize through the existing client-wins SDP
glare rule, including commands received while an offer is awaiting an answer.
No `pingo-proto` or backend/UI contract changes were needed.

## Native video verification (2026-10-25)

Resumed from `t3/native-video-sharing-1`, including the unverified `c1bea20`.
The follow-up fixes retain a static screen for PLI refreshes, close portal
sessions when startup is cancelled, and withhold remote deltas until a keyframe
provides decoder references. System audio can now mix the safe application
monitor streams. Encoder startup failure is checked without assuming ICE
Connected also means the SCTP data channel is ready.

The full-server video test now runs in-process by default, without a prebuilt
server binary. `tests/backend-fixture` is a test-only library importing the real
backend modules: schema, auth, `voice:join`, environment lookup, `/rtc`, and voice
state hooks all execute production code. This avoids copying a fake admission
implementation or changing the server's binary-only package. TestApp creates
two users, and authenticated HTTP calls obtain their media tokens. Leave hooks
must clear backend voice state after each mode. Expensive PSNR comparisons run
outside the live RTC executor; small 192×108 protocol fixtures allow unoptimized
builds to test cadence independently of full-size encoder throughput.

Verified commands and outcomes:

- `cargo test --workspace`: **76 passed, 2 ignored**, no failures.
- `cargo test -p pingo-media --features linux-capture,vp8,vaapi`:
  **34 passed, 4 ignored**, no failures. This includes conversion, PSNR,
  layer sizes, static-screen PLI recovery, camera, stereo ScreenAudio,
  renegotiation, and the in-process backend test.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo clippy -p pingo-media --features linux-capture,vp8,vaapi --all-targets -- -D warnings`,
  and `cargo fmt --all --check`: **passed**.
- `cargo check -p pingo-media --features linux-capture,vp8,vaapi --examples`:
  **passed**.
- `cargo build -p pingo-server`: **passed**, before browser tests.
- `cd apps/web && bun run test:e2e`: **3 passed, 1 failed**, reproduced twice.
  The audio/screen/call case passes. The unrelated history case at
  `apps/web/tests/e2e/real/chat.spec.ts:76` cannot find “History message 0” after
  loading earlier conversations and pressing Control+Home. UI files were not
  changed; generated screenshot changes were discarded.
- The ignored VAAPI encode/decode test: **30/30 frames decoded**, including a
  requested keyframe. An invalid `PINGO_VAAPI_DEVICE=/dev/does-not-exist` selected
  OpenH264 for both layers and encoded all 16 input frames in a one-second run.
- The real portal example and ignored capture test both timed out after **45 s**
  waiting for picker approval. No real screen frames or real-capture performance
  measurements were obtained. Cancellation closes the pending portal session.

The in-process SFU and full RxFerrix cases each decoded **60 Text frames at
15 fps** and **240 Motion frames at 60 fps**, with minimum sampled full-YUV
PSNR **40.07 dB** and **43.61 dB**, respectively, at the selected 96×54 low
layer. Camera delivered 50 frames in its 25 fps fixture; independent 440/880 Hz
ScreenAudio channels remained separate (amplitudes 0.105/0.104). Software codec
round trips at 320×180 measured minimum PSNR **39.46/39.76 dB** for H.264
Text/Motion and **57.36/48.23 dB** for VP8.

### Chromium interoperability

After installing the web dependencies, build the server and feature-enabled
example, then run:

```sh
cargo build -p pingo-server
cargo build --release -p pingo-media --features linux-capture,vp8,vaapi --example screenshare
PINGO_NATIVE_PUBLISHER="$PWD/target/release/examples/screenshare" node crates/pingo-media/tests/browser_video.mjs
PINGO_NATIVE_PUBLISHER="$PWD/target/release/examples/screenshare" node crates/pingo-media/tests/browser_video.mjs --vp8
PINGO_NATIVE_PUBLISHER="$PWD/target/release/examples/screenshare" node crates/pingo-media/tests/browser_video.mjs --hardware
```

**All six cases passed**: Text and Motion with OpenH264, libvpx VP8, and
VAAPI H.264. Each creates a real Chromium RTCPeerConnection against RxFerrix,
publishes a synthetic screen with the production native Client, and requires
at least 20 decoded frames, the expected codec, a playing 320×180 or 640×360
`<video>`, and no signaling/playback errors. Short Chromium `framesPerSecond`
samples include startup/pacing bursts; they are not sustained throughput results.

### Detected support and measurements

The host runs Hyprland with `wayland-1` and a reachable PipeWire daemon. No
`Video/Source` camera node was present. Its GPU is **AMD Navi 48, Radeon RX
9070/9070 XT/9070 GRE** (`1002:7550`), with `/dev/dri/renderD128`. FFmpeg exposes
`h264_vaapi` and `libvpx`; actual VAAPI operation was verified, not inferred only
from the encoder list. FFmpeg also lists NVIDIA encoders, but this host has no
NVIDIA GPU and NVENC is not integrated.

Five-second release runs used `screenshare --capture-only --synthetic`, with
both simulcast layers, conversion and scaling. CPU is process user+system time
divided by wall time, as a percentage of **one CPU core**; it includes synthetic
frame generation and excludes build time. These are synthetic capture results:

- OpenH264, **3840×2160 Text**: **3.70 encoded fps**, **239.87 ms/source frame**,
  **87.1% CPU**. Frame skipping reduced 29 captured frames to 19 encoded frames.
- OpenH264, **1920×1080 Motion**: **27.73 encoded fps**, **32.12 ms/source frame**,
  **94.7% CPU**. 145 captured / 139 encoded frames.
- VAAPI, **3840×2160 Text**: **15.09 fps**, **31.32 ms/source frame**,
  **45.5% CPU**. All 76 captured frames encoded.
- VAAPI, **1920×1080 Motion**: **60.01 fps**, **11.56 ms/source frame**,
  **59.5% CPU**. All 301 captured frames encoded.

Use `--width 3840 --height 2160` for Text, or
`--motion --width 1920 --height 1080` for Motion; append `--software` to force
OpenH264. Encoding time covers all layers; it is not time per individual layer.
Software fallback does not meet the maximum requested rates on this host.

Remaining limits: portal picker approval and real camera/app-monitor capture
need interactive verification; system audio takes an application-stream snapshot
at start; cameras require a packed-RGB PipeWire adapter. DMA-BUF import,
publisher dynacast, macOS/Windows capture implementations, and NVENC remain
unimplemented. STUN/TURN and reconnect are existing transport limits. See the
encoding section above for source-built OpenH264 patent/bundling obligations and
the optional system FFmpeg build's GPL/LGPL implications. No shared signaling,
backend or UI contracts changed; the root Cargo.lock is committed.

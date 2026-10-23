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
  suppression disabled** because RNNoise follows it. Studio bypasses APM.
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
  Native clients are verified against the embedded real SFU, including wildcard
  client/server sockets. STUN/TURN deployment, ICE-TCP, native Opus RED, reconnect
  and browser interoperability remain future work. Only 10/20 ms remote Opus
  packets are accepted.
- Hardware hot-plug, acoustic listening quality and real microphone-to-speaker
  latency have not been tested. Seeded 5% loss plus bursts is exercised, but this
  does not establish the product's subjective “no audible artifacts” target.

## Verification

```sh
cargo test --workspace
cargo test -p pingo-media --no-default-features
cargo test -p pingo-media --features deepfilter
cargo check -p pingo-media --examples
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo test -p pingo-media --test aec -- --nocapture
```

The default media suite has 20 hardware-free tests, plus ignored hardware and
release-timing tests. Coverage includes AEC convergence/double-talk, DC rejection,
RNNoise, resampling, Opus profiles/PLC/FEC, runtime controls, mixing, RTP wrap,
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

Capture workers own their PipeWire loops; video queues are bounded and return
the newest frame. Timestamps use PipeWire header PTS relative to the first frame
(with monotonic receipt time fallback); damage is exposed when supplied.
Only BGRx/RGBx CPU-mapped buffers are negotiated. DMA-BUF import is **not
implemented**, and no modifier format is advertised. There is no GPU zero-copy.
Cameras enumerate PipeWire `Video/Source` nodes by stable object serial. A camera
must offer packed RGB through its PipeWire adapter; native MJPEG/YUYV conversion
is not yet provided. macOS ScreenCaptureKit and Windows.Graphics.Capture remain
explicit platform TODO modules.

`AppAudioCapturer` captures one application's playback ports as 48 kHz stereo
10 ms PCM frames. Select from `list_sources()`, which excludes this process,
Pingo-named nodes and nodes with unknown ownership. The selected serial is
revalidated at start and reconnection is disabled. A whole output-monitor mix
is deliberately not exposed because it would include call playback; separate
sink routing is required before implementing that option. Feed these frames to
the RTC screen-audio input, which encodes stereo Opus without microphone DSP.

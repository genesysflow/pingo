# Pingo desktop

Tauri 2 hosts the shared Svelte UI from `apps/web`. In development it starts Vite on port 5173; release
builds embed `apps/web/dist`. The first launch asks for a server origin; it is remembered in
`$XDG_CONFIG_HOME/chat.pingo.desktop/preferences.json`. Change it under Settings → Keyboard shortcuts.
Auth stays in the webview's local storage, namespaced by server URL by RxFerrix. No token is passed in a URL.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/desktop
bun install
bun run tauri dev
bun run tauri build
```

## Servers, CORS and auth

The webview calls the selected origin's `/api` HTTP endpoints and sync WebSocket directly. Native calls
use that same origin's `/rtc`. RxFerrix 0.6.1 accepts `setupRxFerrix(serverUrl)`; its client sends bearer
tokens and its auth storage key is `rxferrix.auth:${client.url}`. Cross-origin cookies are unnecessary.

RxFerrix defaults to permissive CORS. When restricted, set **Dashboard → Settings → CORS origins**
(`corsOrigins` in RxFerrix `_settings`) to include your web origins and `tauri://localhost` (Linux/macOS),
`http://tauri.localhost` (Windows), and `http://localhost:5173` (development).
This is the real option in RxFerrix `crates/rxferrix/src/settings/mod.rs`; `crates/rxferrix/src/server.rs::cors` consults
`app.auth().cors_allows`. No invented environment variable or backend contract change is required.
Use HTTPS for servers outside your local network. The SFU still needs reachable UDP; HTTP CORS does not
configure ICE, NAT or TURN.

Password auth and email-code entry use the normal API. OAuth/magic-link browser return to a packaged
webview is not implemented yet; use password or email code. `pingo://invite/CODE` opens the shared invite
flow without interpreting arbitrary URLs. A running instance receives new invitations through the
single-instance/deep-link plugins; a cold-start invitation is queued until the UI is ready.

## Desktop integrations

The tray has Open, Mute and Quit. Window size/position persist through the window-state plugin.
Incoming mentions and DMs use native notifications when the window is unfocused; the initial inbox
snapshot is not notified. Existing notification preferences apply. The global shortcut is configurable
under Settings → Keyboard shortcuts, with an empty shortcut disabling it.

The global-shortcut plugin uses platform APIs/X11 on Linux; Wayland compositors (including Hyprland)
may refuse registration or only deliver keys to XWayland windows. In-app push-to-talk remains the fallback.
If the plugin cannot initialize (including sessions without X11), Pingo logs the failure and continues
with in-app push-to-talk. Pingo does not edit compositor bindings or claim system-wide Wayland support.

The updater plugin is installed with empty endpoints/key, and updater artifacts/checks are disabled.
Before enabling releases, supply an HTTPS release feed, a real public signing key and signed artifacts;
set `bundle.createUpdaterArtifacts` and explicitly add a check/install UI.

WebKitGTK GPU workaround: if the window is blank, try `WEBKIT_DISABLE_DMABUF_RENDERER=1 pingo`. The
environment must be set before WebKit starts; it can reduce rendering performance. NVIDIA plus Wayland
is a known problematic combination. Startup logs include the renderer workaround and GDK backend settings.
On this machine, native Wayland delivered IPC but never ran video animation callbacks;
`GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 pingo` works through XWayland under Hyprland.
Keep hardware acceleration enabled on working systems.
Linux requires WebKitGTK 4.1, GTK3, ALSA/libopus and an AppIndicator implementation for the tray.
macOS and Windows bundle settings and macOS microphone/camera usage text exist but are unverified.

## Native calls and sound check

`CallManager` owns a bounded command queue and a dedicated current-thread Tokio worker. The worker owns
CPAL, DSP, Opus and `pingo-media::rtc::Client`; no WebRTC/audio capture runs in the webview. The shared
UI selects `TauriCallController` through Tauri's runtime detection and continues obtaining `voice.join`
admission itself. Every join/retry uses a fresh token. Native events carry session IDs so late disconnects,
old joins and stale commands cannot affect a replacement call. Five bounded exponential reconnect attempts
match the browser policy. A 15-second watchdog catches stalled setup.

Mute, deafen, push-to-talk, Off/Gentle/Strong suppression, Studio, AEC, device selection and per-user
volume/pan feed native `AudioSettings`. Lounge distance gain multiplies the user's volume. The tray and
global shortcut operate the native manager directly. Device lists refresh every three seconds, while
stream recovery remains inside pingo-media. Native event overflow requests reconnect instead of leaving
an incomplete participant map. Quality includes loss, jitter, RTT and current playout delay.

Sound check lists CPAL devices and runs mic → send DSP/Opus → receive jitter/mix → selected speakers for
five seconds, showing the resulting level. It does not use WebKit's microphone or recording APIs. Leave
an active call before using loopback. Duplicate device names retain pingo-media's existing ambiguity.
Native screen/camera **sending** is intentionally left to the concurrent pingo-media video implementation;
unsupported controls show an error rather than simulating a successful share.

## Video display spike

The Stage uses **native FFmpeg decoding → JPEG images over a binary Tauri Channel → canvas** on this
Linux build. A bounded worker depacketizes H.264/VP8 RTP and drives FFmpeg; it is an external decoder
process, not a WebRTC pipeline inside WebKit. Output is capped at 1920 pixels wide. The raw RGBA prototype
is retained for comparison. There is no wgpu overlay yet. FFmpeg must be on PATH; `.deb` declares it as a
dependency, while AppImage users must install it separately.

WebKitGTK 2.54.1 exposes `VideoDecoder`, but `isConfigSupported` returns **false for both H.264 and VP8**.
The encoded Channel delivered 94,774 H.264 bytes and 74,834 VP8 bytes; WebCodecs cannot decode them on
this host. The probe contains a real VideoDecoder/canvas path for webviews that support the requested
codec. Its encoded check sends a keyframe; throughput measurements below use native decode because
neither WebCodecs configuration works here. Other platforms have not been measured.

Run the probe in the actual embedded webview, with FFmpeg/libx264/libvpx available:

```sh
cd apps/desktop
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 bun run video:probe
# Reuse an app built by `tauri build --debug --no-bundle`:
PINGO_SKIP_BUILD=1 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 bun run video:probe
```

`cargo test --workspace` may replace `target/debug/pingo` with a binary configured for the Vite dev URL.
Rebuild with `tauri build --debug --no-bundle` before using `PINGO_SKIP_BUILD=1`; otherwise a missing Vite
server looks like a renderer/probe timeout. The launcher removes stale reports and records failure metadata.

Fixtures contain five seconds of 1920×1080 testsrc2 at 30/60 fps. The October 25 rerun is in
`benchmarks/webkit-xwayland-current.json`: Ryzen 7 5800X3D, WebKitGTK 2.54.1, debug build, XWayland,
DMABuf workaround enabled, shared build machine. H.264 → JPEG displayed **134/150** frames at 30 fps
and **210/300** at 60 fps (16/90 dropped), using **66.4%/130.6%** of one CPU core, with median
IPC-to-canvas delay **13.2/12.1 ms** (p95 21.2/21.4 ms). VP8 → JPEG displayed **135/150** and
**226/300** (15/74 dropped), at 68.1%/111.6% CPU and 13.0/11.4 ms median (p95 20.7/20.4 ms).
Raw RGBA was substantially worse: H.264 displayed 87/150 and 113/300 at 180.4%/218.4% CPU,
with median delay 33.5/29.7 ms. All eight samples and both codec-support results are recorded.

`benchmarks/webkit-wayland-current.json` records the same current binary on native Wayland: each
sample received one binary frame but ran **zero animation callbacks**, displaying zero frames. The
probe deliberately fails this case. The inherited October 23 comparison, earlier raw run, and earlier
Wayland failure remain in `webkit-video.json`, `webkit-raw.json` and `webkit-wayland-stalled.json`.

CPU sums the app and its current child processes using Linux `/proc` user/system ticks (100 Hz).
Latency starts **after native decode** and ends after the canvas draw call; it excludes network,
decode time and physical display scanout. Drops count frames skipped by bounded IPC backpressure;
displayed counts successful draw calls, not physical scanouts. This is a display spike, not a
mouth-to-eye measurement. The JPEG fallback is usable for Stage display but does not sustain 1080p60.
A native GPU surface remains the next experiment for high frame rates.

Only one frame per surface may be awaiting acknowledgement, so a hidden or stalled webview cannot
accumulate raw images. Subscription IDs isolate late acknowledgements/unsubscribes from replacement
tiles. Decode queues, frame dimensions, payload sizes and the number of surfaces are bounded.

Additive pingo-media receive API from the merged work: `RemoteVideoPacket` (track, codec, sequence,
RTP timestamp, marker and encoded payload), `RtcEvent::RemoteVideo`, and `Command::VideoQuality`.
The client negotiates H.264/VP8 receive support, associates RTP with published tracks, sends quality
preferences and requests a keyframe on resume. These are library additions; `pingo-proto` and
`docs/API.md` are unchanged. This resumption makes no further edits to `rtc.rs`.

## Verification and packaging

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo build -p pingo-server
(cd apps/web && bun run check && bun run test && bun run build && bun run test:e2e)
(cd apps/desktop && bun run tauri build)
# Extra local integration checks (no audio hardware):
cargo test -p pingo-desktop --test real_server -- --ignored --nocapture
cargo test -p pingo-desktop --lib -- --ignored --nocapture
```

The first explicit integration test launches a temporary development backend and drives CallManager plus a
second native client with synthetic audio. It requires `target/debug/pingo-server` (or `PINGO_SERVER`). The
second checks live H.264/VP8 RTP depacketization and FFmpeg decoding before EOF. Both are ignored in the
ordinary hardware-free suite because they require external binaries.

The GUI smoke uses the repository's WebKitWebDriver workflow. It signs up through the real bundled UI,
creates a space, joins a second native Rust peer, asserts non-silent decoded PCM and two Stage participants,
and saves `screenshots/native-call.png` plus `screenshots/smoke.json`. Optional video adds a Chromium
publisher and asserts decoded pixels in the native Stage and removal after unpublish. Only screen capture
is replaced with an animated canvas; signaling, SFU forwarding and native decoding are real.

```sh
cd apps/desktop
# Requires WebKitWebDriver, FFmpeg and the web package's Playwright Chromium:
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 PINGO_SMOKE_VIDEO=1 bun run smoke
```

The smoke's debug-only `PINGO_DESKTOP_TEST_HEADLESS=1` bypasses hardware opening in the desktop client;
release builds ignore it. For a manual acoustic check, run the app normally, select your server, sign in,
open Sound check, choose devices and test with headphones. Join the same room from a second native client,
verify both tiles and audio, then test mute/deafen, hold Space with push-to-talk enabled, switch devices,
and leave. Check native notifications with the window unfocused and open `pingo://invite/CODE` from your
system. These physical-device and desktop integration checks are not asserted by the synthetic smoke.

Linux release targets are AppImage and `.deb` under `target/release/bundle/`. GStreamer plugin bundling is
disabled because calls and video use native media; GTK/WebKit dependencies still bundle automatically.
This also avoids the optional GStreamer bundler's external `patchelf` requirement on this machine.
macOS and Windows settings exist; build on those hosts with `bun run tauri build --bundles dmg` or `--bundles nsis` to override the Linux
target list. Both are unverified. Updater artifacts remain disabled, native screen/camera sending belongs
to the parallel media work, and OAuth browser return remains unimplemented.

Verified on 2026-10-25 after resumption: `cargo test --workspace` passed 77 tests (four explicitly ignored
external/hardware checks); `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` passed. Web `bun run check` reported zero diagnostics, `bun run test` passed
33 tests, `bun run build` succeeded, and `bun run test:e2e` passed all four real-server suites after
`cargo build -p pingo-server`. The explicit native-manager integration passed with 318,720 non-silent
PCM bytes, 45 Opus packets and two durable participants; the live H.264/VP8 decoder integration passed.
`bun run tauri build --debug --no-bundle` and release `bun run tauri build --verbose` succeeded.
Release artifacts are `Pingo_0.1.0_amd64.AppImage` (112.93 MiB) and `Pingo_0.1.0_amd64.deb` (11.57 MiB).
Cargo.lock is committed, including the merged desktop dependencies.

The debug XWayland GUI smoke passed with 314,880 non-silent PCM bytes and 42 remote Opus packets.
The **release AppImage** GUI smoke then passed with **334,080 PCM bytes, 45 remote Opus packets, two
native participants, a decoded 640×360 remote screen, and successful unpublish cleanup**. Current
screenshots are `screenshots/native-call.png` and `screenshots/native-screen.png`; counters and pixel
evidence are in `screenshots/smoke.json`. I inspected the Stage screenshot: participant tiles and the
purple remote screen rendered correctly in the packaged webview. The first packaged video smoke failed
while the publisher reconnected; the harness now waits for its connected state before sharing and confines
the ALSA test override to the desktop process. FFmpeg errors and browser publisher errors are retained in
smoke logs. Chromium's speaker output is muted for the automated run.

The release run opens an ALSA null device; it does not rely on the debug hardware bypass. Reproduce from
`apps/desktop` after building the server and native peer as above:

```sh
cat > /tmp/pingo-null-alsa.conf <<'EOF'
pcm.!default { type null }
EOF
PINGO_SKIP_BUILD=1 PINGO_SMOKE_VIDEO=1 \
  PINGO_DESKTOP_BINARY="$PWD/../../target/release/bundle/appimage/Pingo_0.1.0_amd64.AppImage" \
  PINGO_DESKTOP_ALSA_CONFIG=/tmp/pingo-null-alsa.conf \
  GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 bun run smoke
```

These runs use synthetic sound; physical acoustics and global Wayland shortcuts remain unverified.
The packaged AppImage also rendered the server picker through WebKitWebDriver
(`screenshots/packaged-launch.png` and `.json`). A separate launch with DISPLAY unset on Wayland stayed
running until the intentional four-second timeout; shortcut availability is still compositor-dependent.

Resumption fixes: isolate video subscriptions and acknowledgements across replaced tiles, release
backpressure after display errors, exercise actual WebCodecs decoding on supported webviews, wait for
sound-check listeners before starting loopback, tolerate unavailable global-shortcut services, log
renderer configuration, preserve probe failures and metadata, disable unused GStreamer plugin bundling,
and make packaged smoke tests wait for the publisher connection and isolate test audio settings.
No additional media API or wire-contract change was needed beyond the received-video additions already in the merged branch.

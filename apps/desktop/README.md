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
This is the real option in RxFerrix `src/settings/mod.rs`; `src/server.rs::cors` consults
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
Pingo does not edit compositor bindings or claim system-wide Wayland support.

The updater plugin is installed with empty endpoints/key, and updater artifacts/checks are disabled.
Before enabling releases, supply an HTTPS release feed, a real public signing key and signed artifacts;
set `bundle.createUpdaterArtifacts` and explicitly add a check/install UI.

WebKitGTK GPU workaround: if the window is blank, try `WEBKIT_DISABLE_DMABUF_RENDERER=1 pingo`. The
environment must be set before WebKit starts; it can reduce rendering performance. NVIDIA plus Wayland
is a known problematic combination. Keep hardware acceleration enabled on working systems.
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

Part B verification: `cargo test -p pingo-desktop` (7 tests),
`cargo clippy -p pingo-desktop --all-targets -- -D warnings`, `bun run check` (zero diagnostics),
`bun run test` (29 tests including five mocked Tauri controller tests), and `bun run build`.
No pingo-media API or wire contract changed for audio integration.

Resumption fixes: removed stray diff markers from the native sound-check wizard and clear remote
video tiles immediately when a participant leaves or a call reconnects. The standalone real-server
smoke now explicitly enables reqwest's TLS provider (previously panicked before HTTP setup).
Verified `cargo test -p pingo-desktop` (11 tests), and
`cargo build -p pingo-server` followed by
`cargo test -p pingo-desktop --test real_server -- --ignored --nocapture`: 334,080 non-silent
decoded PCM bytes, 44 remote Opus packets and two durable participants. Web checks report zero
diagnostics and all 30 unit tests pass.

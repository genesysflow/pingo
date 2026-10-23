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

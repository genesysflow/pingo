# How Pingo fits together

Pingo separates persistent chat data from time-sensitive media. Both services run in the same Rust server process, while the browser and desktop app share a Svelte 5 interface.

## Chat and durable state

`pingo-server` is an RxFerrix application. It owns authentication, spaces, rooms, messages, membership, permissions, files and search. Clients subscribe to reactive queries over a WebSocket; mutations update the embedded SQLite database and subscribers receive new results.

Fast-changing records such as typing, presence, read state and voice membership live in small tables of their own. Queries use narrow index reads, and mutations avoid no-op writes.

The [backend API contract](../../../../docs/API.md) defines the function names and result shapes consumed by the generated TypeScript client.

## Calls and screen sharing

`pingo-sfu` is a media relay built on str0m. It is mounted at `/rtc` in the server and can also run as a standalone binary.

1. A signed-in client calls `voice.join` for a room.
2. The backend checks permissions and returns a short-lived media token.
3. The client connects to `/rtc` over a WebSocket and negotiates WebRTC.
4. Audio and video travel over UDP to the SFU, which forwards tracks to authorized participants. When TURN is configured, `voice.join` also returns short-lived relay credentials, and clients that cannot reach the SFU directly route media through the relay.
5. SFU hooks update durable voice membership in the backend.

Media and signaling types live in `pingo-proto`. Update all consumers together when changing that contract. The [SFU guide](../../../../crates/pingo-sfu/README.md) describes admission, forwarding, simulcast and deployment limits.

## Ephemeral events

Speaking indicators, audio levels, raised hands, call reactions, pointers and drawing strokes use the SFU's `pingo` data channel. They do not write to SQLite. Keeping these events off the database avoids making the chat writer handle events that change several times a second.

## Browser and desktop

The browser uses WebRTC and Web Audio. DeepFilterNet3 runs as WebAssembly in an AudioWorklet. Screen sharing uses the browser's capture APIs.

The Tauri 2 desktop shell hosts the same UI, while `pingo-media` owns native audio capture, processing, Opus, jitter buffering, mixing and WebRTC. On the verified Linux build, received video is decoded by FFmpeg and displayed through a Tauri Channel. Read the [desktop guide](../../../desktop/README.md) for platform support and current limitations.

The [native media guide](../../../../crates/pingo-media/README.md) and [noise-suppression guide](../../../../crates/pingo-denoise/README.md) cover the media pipeline, capture support and measurements.

## Deployment

The self-hosted container includes the backend, embedded SFU and built web app. Caddy is an optional Compose service for HTTPS. Data and uploaded files persist under `/data`.

HTTPS carries the web UI, API, synchronization and call signaling. Media needs its own reachable UDP port, defaulting to `40000`. An optional coturn sidecar (the `turn` Compose profile) relays calls for clients on networks that block direct UDP; the relay itself still reaches the SFU over UDP. Pingo has no ICE-TCP fallback. See [networking for calls](../../../../docs/SELF_HOSTING.md#calls-behind-nat-and-firewalls) and the [optional TURN relay](../../../../docs/SELF_HOSTING.md#optional-turn-relay).

This documentation site is a separate static build. GitHub Pages serves the presentation and guides; a Pingo application server runs on your own infrastructure.

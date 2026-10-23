# Pingo: plan for a Slack/Discord alternative built on RxFerrix

> Working name: **Pingo** (from the project folder). Status: plan, nothing built yet.
> Based on RxFerrix v0.6.x (github.com/genesysflow/rxferrix), read on 2026-10-23.

---

## 1. What we're building

A self-hostable team and community chat app with:

1. **Live messaging**: spaces, rooms, threads, DMs, reactions, files, search, unread state, mentions.
2. **Voice calls** that sound clearly better than Discord or Slack huddles: full-band Opus, ML noise suppression, a "Studio" mode for music.
3. **Screen sharing** at 1080p60, with a sharp "text" mode for code and docs, plus system audio.
4. **A UI that's easy to learn but feels new.** Voice is part of every room instead of a separate channel type, the call controls stay visible while you browse, and there's an optional spatial "Lounge" (details in §6).

**Product pillars:** easy to use, best-in-class audio, one binary to self-host.

---

## 2. What RxFerrix gives us, and what it doesn't

RxFerrix is a reactive backend in Rust, similar to Convex. You write `#[table]`, `#[query]`, `#[mutation]` and `#[action]` in Rust. Clients subscribe to queries over a WebSocket, and their results update when data changes. It ships as a single binary with embedded SQLite.

| Need | RxFerrix | Notes |
|---|---|---|
| Live chat data (messages, unread, members) | ✅ Reactive queries, diffs, shared fan-out | Their `examples/chat-react` is already a small Slack clone. Use it as the reference. |
| Auth | ✅ Password, magic link, OAuth (GitHub/Google/Discord/Apple/OIDC), 2FA, rate limits | Covers sign-in completely. |
| Files and attachments | ✅ Local disk or S3, thumbnails, signed URLs | |
| Search | ✅ FTS5 with stemming in 18 languages, plus vector search, both reactive | Message search comes nearly free. |
| Counts and unread badges | ✅ Live aggregates | |
| Reminders, digests, cleanup | ✅ Scheduler, crons, durable workflows | |
| TS client | ✅ Generated `api` + `@rxferrix/react` / `@rxferrix/svelte` | Optimistic updates and automatic reconnect. |
| Custom endpoints in the same binary | ✅ `RxFerrix::app_routes(axum::Router<App>)` | **This is where the voice/video signaling lives.** Handlers can call `app.mutation::<F>()` and `app.auth()`. |
| Ops | ✅ Backups, point-in-time restore, zero-downtime `--takeover`, Prometheus | |
| **Real-time media (voice/video/screen)** | ❌ | We build an SFU (selective forwarding unit, the server that relays media between call participants). See §4.2. |
| **High-frequency ephemeral state** (speaking indicators, cursors, audio levels) | ⚠️ Every write goes through the single SQLite writer, and invalidation is coarse | Keep this state **off the database**. It goes over the SFU's data channel. |
| Horizontal scaling | ❌ Single node by design | Fine for self-hosted communities. Media (the expensive part) can be split onto separate nodes later (§4.2). |
| Offline-first | ❌ | Not needed for v1. |

Measured RxFerrix limits that matter here: about 9–18k mutations/s on one writer, 1,024 live queries per socket, 5 s mutation timeout, no network I/O inside mutations.

---

## 3. Architecture

```text
┌──────────────── Desktop client (Tauri 2) ─────────────────┐      ┌──────── Browser client (guest / web) ────────┐
│  UI: Svelte 5 + @rxferrix/svelte (same code as web)       │      │  Same UI, browser WebRTC for media           │
│  ───────────────────────────────────────────────────────  │      │  DeepFilterNet (WASM) in an AudioWorklet     │
│  pingo-media (native Rust, in the Tauri core process):    │      └──────────────┬───────────────────────────────┘
│   cpal I/O → AEC/AGC (webrtc-audio-processing)            │                     │
│   → DeepFilterNet NS → Opus 48k → str0m (WebRTC client)   │                     │
│   screen capture (PipeWire portal / SCK / WGC) → H.264/AV1│                     │
└───────────────┬───────────────────────┬───────────────────┘                     │
                │ WSS: rxferrix sync    │ WSS /rtc signaling + UDP/TCP media      │
                ▼                       ▼                                         ▼
┌──────────────────────────── pingo-server (one binary) ──────────────────────────────────────────┐
│  RxFerrix runtime: queries · mutations · actions · auth · storage · search · scheduler · /_/    │
│  app_routes:  /rtc  ──►  pingo-sfu (str0m, sans-IO)                                             │
│                           rooms → participants → tracks (simulcast, BWE, top-N audio)           │
│                           on join/leave → app.mutation::<voice::set_state>()                    │
│  embed_static: built web UI                                                                     │
│  SQLite (WAL) data dir · S3/disk for files · coturn sidecar (optional TURN)                      │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Data paths:**

- **Durable state** (messages, rooms, membership, who's in which call, mute/deafen) goes through RxFerrix mutations, and clients see it through reactive queries.
- **Ephemeral state** (speaking indicators, audio levels, screen-share pointer, reactions-in-call) goes over the SFU data channel. It never touches SQLite.
- **Media** goes over WebRTC (SRTP over UDP, with ICE-TCP or TURN fallback) to the SFU.

---

## 4. Components

### 4.1 Backend: `pingo-server` (the RxFerrix app)

**Data model** (Rust structs are the schema):

```rust
#[table] #[index(by_slug = [slug])]
pub struct Space { pub name: String, pub slug: String, pub icon: Option<Id<Storage>>, pub accent: String, pub owner: Id<User> }

#[table] #[index(by_space_user = [space, user])] #[index(by_user = [user])]
#[aggregate(members_per_space, group = [space])]
pub struct SpaceMember { pub space: Id<Space>, pub user: Id<User>, pub roles: Vec<Id<Role>>, pub nick: Option<String> }

#[table] #[index(by_space = [space])]
pub struct Role { pub space: Id<Space>, pub name: String, pub color: String, pub perms: u64 /* bitflags */, pub position: i32 }

/// One concept: a Room has chat AND an always-available call. `kind` only changes defaults.
#[table] #[index(by_space_pos = [space, position])]
pub struct Room { pub space: Id<Space>, pub name: String, pub topic: String, pub kind: RoomKind /* Chat | Lounge | Stage | Dm */,
                  pub private: bool, pub position: i32, pub audio_profile: AudioProfile /* Voice | Studio */ }

#[table] #[index(by_room = [room])] #[index(by_thread = [thread_root])]
#[search_index(search_body, search = body, filter = [room])]
#[aggregate(messages_per_room, group = [room])]
pub struct Message { pub room: Id<Room>, pub author: Id<User>, pub body: String, pub thread_root: Option<Id<Message>>,
                     pub attachments: Vec<Id<Storage>>, pub edited_at: Option<f64>, pub deleted: bool }

#[table] #[index(by_room_user = [room, user])]   // small, fast-changing tables, kept separate (per RxFerrix guidance)
pub struct ReadState { pub room: Id<Room>, pub user: Id<User>, pub last_read_at: f64 }
pub struct Typing    { /* room, user, expires_at: scheduled delete, as in chat-react example */ }
pub struct Presence  { /* user, status: Online|Idle|Dnd|Offline, custom text; 60 s heartbeat + scheduled expiry */ }
pub struct VoiceState{ /* room, user, muted, deafened, sharing_screen, joined_at; written ONLY by the SFU */ }
pub struct Mention   { /* user, message, room, seen: feeds the "Catch up" inbox */ }
pub struct Invite    { /* space, code, uses, expires_at */ }
pub struct Reaction  { /* message, user, emoji */ }
```

**Function modules:** `spaces`, `rooms`, `messages` (send/edit/delete/paginate/thread), `reactions`, `read_state`, `typing`, `presence`, `search`, `inbox` (mentions/replies/DMs in one feed), `invites`, `roles` (permission checks in a shared `require_perm(ctx, space, PERM)` helper, since RxFerrix has no declarative rules), `voice` (issue a media token, read voice states), `notifications` (an action that sends web push and email digests through scheduler/workflows).

**Rules we follow because of RxFerrix's design:**

- Fast-changing fields live in small tables of their own (`Typing`, `Presence`, `ReadState`, `VoiceState`), never on `Room` or `Message`.
- Queries stay narrow: index `eq` prefixes, `take(n)`, `_creationTime` bounds, and `.count()` / aggregates for badges.
- Skip no-op writes, as `mark_read` does in the example.
- Presence heartbeats are **coarse** (60 s), and expiry runs through `scheduler.run_at`. The live "speaking" pulse never comes from the DB.
- Messages use `usePaginatedQuery`, so new messages only invalidate the newest page.

### 4.2 Media server: `pingo-sfu`

**Choice: our own SFU on [`str0m`](https://github.com/algesten/str0m)**, a sans-IO Rust WebRTC library that runs in production. It runs in the same binary, mounted via `app_routes("/rtc")`.

- **Why not LiveKit or mediasoup?** They're Go and C++/Node sidecars, which breaks the "one Rust binary" story and limits control over the audio path. **LiveKit stays as the fallback** if the SFU spike (Phase 0) goes badly. The client signaling layer is abstracted so we could swap.
- **Join flow:**
  1. The client calls mutation `voice.join(room)`. It checks permissions and returns a short-lived signed media token (room, user, can_speak, can_share, expiry; HMAC key from `ctx.env`).
  2. The client opens `wss://…/rtc?token=…`. The SFU verifies the token, then does SDP/ICE.
  3. The SFU calls `app.mutation::<voice::set_state>` on join, leave, mute and share. Everyone's sidebar updates live through normal reactive queries.
  4. If the socket drops, the SFU writes the leave. A cron sweeps stale `VoiceState` rows after a crash.
- **Features:**
  - One UDP port for all peers (ICE ufrag demux).
  - ICE-TCP on 443, plus an optional coturn sidecar for strict corporate networks.
  - Per-subscriber bandwidth estimation (TWCC), simulcast layer switching (3 layers for camera, 2 for screen), and pausing layers nobody watches ("dynacast").
  - **Top-N audio forwarding.** It uses the RFC 6464 audio-level header to forward only the N loudest speakers (N≈5) in big rooms, so 200-person rooms stay cheap.
  - Opus RED (redundant audio) turned on for subscribers with packet loss.
  - Data channel for ephemeral events: speaking, levels, raised hand, in-call reactions, screen pointer.
- **Scale path:** each room is a tokio task, so the SFU is a library crate with an optional standalone `pingo-sfu` binary. When one box isn't enough, run media nodes separately. `voice.join` then picks a node and signs the token for it; the database stays single-node.

### 4.3 Clients

**UI stack:** Svelte 5 + Vite + `@rxferrix/svelte`, with the generated `api` written to `web/src/rxferrix`. One UI codebase serves both the web and desktop clients.

- Svelte 5 is light and makes the animated, playful UI cheap to build, and the RxFerrix dashboard uses it too. React is equally supported if the team prefers it (see §9).
- Rich text: TipTap/ProseMirror with markdown shortcuts, mentions, emoji and code blocks.
- Virtualized message list and optimistic sends (the RxFerrix pattern).

**Desktop (main target): Tauri 2 with a native Rust media engine (`pingo-media`)**

- Media runs in Rust in the Tauri core process, not in the webview. This matters because:
  - WebKitGTK, the webview Tauri uses on Linux, has missing or disabled WebRTC on most distros, and we're on Linux.
  - Owning the whole audio pipeline is how we deliver "quality sound" (§5).
- The webview only shows UI. Video frames reach it as **encoded** chunks over a Tauri `Channel` and are decoded with WebCodecs. If a platform's webview lacks WebCodecs, the fallback is a native wgpu overlay surface. **Phase 0 spike decides this.**
- Push-to-talk global hotkey, tray icon, native notifications and auto-update all come from Tauri plugins.

**Web (guests, zero install):** uses the browser's own WebRTC stack against the same SFU.

- Easy onboarding: an invite link drops a guest straight into a room.
- Noise suppression runs DeepFilterNet compiled to WASM in an AudioWorklet.
- Studio mode turns off the browser's echo cancellation, noise suppression and auto gain (`echoCancellation/noiseSuppression/autoGainControl: false`) and sends `stereo=1; maxaveragebitrate=256000`.

**Mobile:** later (Phase 5). Tauri 2 mobile or native shells, reusing `pingo-media`.

### 4.4 Audio pipeline (the quality story)

```text
mic (cpal, 48 kHz, 10 ms buffers)
 → AEC3 echo cancel + AGC2 (webrtc-audio-processing crate)       [off in Studio mode]
 → noise suppression: DeepFilterNet (Rust, full-band 48 kHz)      [Off / Gentle / Strong; off in Studio]
 → voice activity detection + RFC 6464 level → push-to-talk / voice-activation gate
 → Opus 48 kHz, 20 ms frames (10 ms "low latency" option), in-band FEC, DTX
     Voice:  mono, 48–64 kbps, VBR      Studio: stereo, 128–256 kbps, CELT/music mode
 → str0m → SFU
receive:
 SFU → adaptive jitter buffer (own implementation, NetEQ-style: target delay from measured jitter,
       time-stretch/compress via WSOLA, packet-loss concealment from Opus) → per-user volume + loudness
       normalization (EBU R128-style) → optional HRTF spatial panning (Lounge) → mixer → cpal out
```

Quality targets:

- Mouth-to-ear latency **under 150 ms** on a normal broadband connection.
- No audible artifacts at 5% packet loss (FEC + RED).
- Mic test with loopback playback.
- A live "Your audio: Great / OK / Poor" label in plain language, derived from loss, jitter and RTT.

**Screen-share audio:**

- Linux: PipeWire app/monitor stream.
- macOS: ScreenCaptureKit audio.
- Windows: WASAPI process loopback.

It's sent as a separate stereo Opus track, and the sharer's own call audio is excluded.

### 4.5 Screen sharing

- **Capture:**
  - Linux: xdg-desktop-portal ScreenCast + PipeWire, which works on Hyprland via xdg-desktop-portal-hyprland.
  - macOS: ScreenCaptureKit.
  - Windows: Windows.Graphics.Capture.
  - Plan to wrap per-OS code behind one trait. Evaluate the `scap` crate first.
- **Encode:**
  - Hardware first: VAAPI, NVENC, VideoToolbox, Media Foundation.
  - Software fallback: openh264.
  - AV1 (SVT-AV1 / hardware) as an opt-in for screen content later.
- **Two modes, picked by the user (default: auto):**
  - **Text:** up to 4K at 5–15 fps, high quality, screen-content tuning. Code stays crisp.
  - **Motion:** 1080p60 for games and video.
- **Simulcast:** 2 layers, so viewers on bad networks get the lower one and don't drag everyone down.
- **Interactive extras** (data channel): viewers can **point** (a laser dot with their name) and **draw** short-lived annotations on the shared screen. Pop the share out into its own window, or picture-in-picture.

---

## 5. Security and privacy

- TLS everywhere (RxFerrix has built-in `--https` / Let's Encrypt). Media is DTLS-SRTP.
- Authorization lives in function code through one `require_perm` helper. Tests use `TestApp` with `as_user()` for every permission path.
- Media tokens are short-lived (60 s to connect) and scoped to one room with explicit capabilities.
- RxFerrix rate limits cover auth. App-level limits for sends and uploads are counted in a small table with scheduled resets.
- **E2EE for DMs and private calls** comes in Phase 5: SFrame on media (native) or Insertable Streams (web). Message E2EE (MLS) is a separate big project and is out of scope for v1.

---

## 6. UI/UX concept

Design goal: **someone who has never used Discord understands it in 30 seconds, and a Discord power user finds nothing missing.**

**Layout (desktop):**

```text
┌────┬──────────────────┬──────────────────────────────────────────┬───────────────┐
│ ●  │  ACME            │  # design                     🔍  ⓘ     │  Thread /     │
│ ◆  │  ───────────     │  ─────────────────────────────────────── │  Members /    │
│ ▲  │  ⌂ Catch up  (3) │  messages…                               │  Call stage   │
│ +  │  # general       │                                          │  (slides in)  │
│    │  # design  🔊●●● │                                          │               │
│    │  # random        │                                          │               │
│    │  ✦ Lounge 🔊●●   │  [ composer — / commands, @, :emoji: ]   │               │
│    │                  │                                          │               │
│    │ ┌──────────────┐ │                                          │               │
│    │ │🎙 design 0:42│ │  ← the Call Pill: always visible while   │               │
│    │ │ 🔇 🎧 🖥  ✕  │ │    you browse other rooms                │               │
│    │ └──────────────┘ │                                          │               │
└────┴──────────────────┴──────────────────────────────────────────┴───────────────┘
```

**Ideas that set it apart:**

1. **Every room is chat + call.** There are no separate voice channels. The sidebar shows tiny live avatars next to any room with people in a call. One click on "Hop in" joins; no ringing, no setup. A room's call can expand into the main area (stage) or collapse to the pill.
2. **The Call Pill.** A floating mini-panel that stays visible wherever you navigate. It has mute, deafen, share, leave, a live speaking ring, and your connection quality in plain words. Drag it anywhere, or pop it out as an always-on-top mini window.
3. **Lounge rooms (spatial voice).** An optional room type shown as a 2D canvas. Avatars sit on it, and you drag yourself near people to hear them louder. Panning uses HRTF, so voices come from their direction. Several conversations can happen in one room, like at a real hangout. Normal rooms stay a simple grid.
4. **Catch up inbox.** One calm feed of mentions, replies, DMs and threads you follow, with "mark all done". It replaces a wall of red badges, and room badges stay subtle.
5. **Command palette (Ctrl/Cmd+K)** for everything: jump to rooms and people, start a call, share a screen, change your status, search. Keyboard-first throughout, with a visible shortcut sheet (`?`).
6. **Sound settings in plain language:**
   - A first-run "sound check" wizard picks the best mic, sets levels and plays your voice back.
   - The noise slider is labeled "Off · Gentle · Strong", and a separate **Studio** toggle is for musicians and podcasters.
   - Per-person volume is right on their avatar.
7. **Screen share with pointing and drawing.** Viewers can point at things instead of saying "no, the other button".
8. **Spaces have personality.** Each space has an accent color that tints the UI, a custom emoji pack, and an optional banner. Light, dark and high-contrast themes, with per-user density (comfortable or compact).
9. **Motion with purpose.** Speaking rings, a pill that morphs into the stage, threads that slide in as a column. Everything respects `prefers-reduced-motion`.
10. **Accessibility from day 1:** full keyboard navigation, ARIA live regions for new messages, live captions in calls (Phase 5, using a local Whisper model through an action or on the client).

**Design process:** Phase 0 produces clickable Figma (or Svelte prototype) flows for these 5 journeys: first launch, join via invite, send a message, hop into a call, share a screen. Run hallway tests with 5 people before building them for real.

---

## 7. Repository layout

```text
pingo/
├─ Cargo.toml                 # workspace
├─ .cargo/config.toml         # RxFerrix's required SQLite build flags (copied from `rxferrix new`)
├─ crates/
│  ├─ pingo-server/           # RxFerrix app: tables, functions, main.rs (auth, crons, app_routes, embed_static)
│  │  └─ src/{spaces,rooms,messages,reactions,read_state,typing,presence,inbox,roles,voice,invites,notify}.rs
│  ├─ pingo-sfu/              # str0m SFU library (+ optional standalone bin)
│  ├─ pingo-media/            # native media engine: capture, APM, DeepFilterNet, Opus, jitter buffer, video enc/dec
│  └─ pingo-proto/            # signaling + data-channel message types (serde, ts-rs → TS)
├─ apps/
│  ├─ web/                    # Svelte 5 UI (shared); src/rxferrix = generated api
│  └─ desktop/                # Tauri 2 shell; src-tauri depends on pingo-media
├─ deploy/                    # systemd unit, Dockerfile, compose (+ coturn), k3s
└─ docs/
```

---

## 8. Roadmap

Rough sizing assumes **2–3 engineers**. The phases are sequential, but the media track (B) runs in parallel with the chat track (A).

### Phase 0: Spikes and design (2–3 weeks)

Exit criteria (each spike gives a yes/no answer):

- [ ] `rxferrix new` scaffold, plus the chat-react example ported to Svelte and adapted to the "Room" model.
- [ ] **SFU spike:** str0m SFU mounted on `app_routes`. 2 browsers and 1 native client exchange Opus audio, and a data channel works.
- [ ] **Native audio spike:** cpal → DeepFilterNet → Opus → str0m. Measure latency and CPU on Linux, macOS and Windows.
- [ ] **Video-in-webview spike:** WebCodecs decode of H.264 inside Tauri on WebKitGTK, WKWebView and WebView2. Decide between WebCodecs and the wgpu overlay.
- [ ] **Screen capture spike:** PipeWire portal capture on Hyprland/GNOME/KDE, plus SCK and WGC.
- [ ] UI prototype of the 5 core journeys, hallway-tested.

### Phase 1: Messaging MVP (A, 5–6 weeks)

Spaces, rooms, DMs, messages (send, edit, delete, markdown), threads, reactions, attachments, unread counts, typing, presence, invites, roles and permissions, search, the Catch up inbox, the command palette, and the web client deployed as one binary.

Exit: a team of 10 uses it daily instead of Slack for a week.

### Phase 2: Voice (B, in parallel, 6–8 weeks)

- SFU hardened: BWE, top-N audio, RED, ICE-TCP, TURN.
- Native audio pipeline with jitter buffer, PLC and loudness normalization.
- Call Pill, "Hop in", mute/deafen/push-to-talk, per-user volume.
- Sound-check wizard, connection-quality label, Studio mode.
- Tauri desktop app with auto-update.

Exit:

- 25-person call at under 150 ms mouth-to-ear.
- Blind A/B listening test preferred over Discord by most listeners.
- Clean audio at 5% loss.

### Phase 3: Screen sharing and video (B, 5–6 weeks)

- Capture on 3 OSes, hardware encode, Text/Motion modes, simulcast, screen-share audio.
- Pop-out viewer, pointer and annotations.
- Webcam video tiles (same pipeline).

Exit: 1080p60 Motion share and crisp 4K Text share to 10 viewers on a single mid-range server.

### Phase 4: Polish and community features (4–6 weeks)

- Lounge (spatial) rooms and Stage rooms (many listeners).
- Notifications: desktop, web push, email digests via workflows.
- Custom emoji, theming, moderation tools (timeouts, audit via RxFerrix audit log, slow mode).
- Bots and webhooks via `app_routes` and an HTTP API.
- Import from Slack/Discord exports via a workflow.

### Phase 5: Beyond v1

Mobile apps, E2EE DMs and calls (SFrame), live captions, recording, separate media nodes for scale, federation (maybe).

---

## 9. Risks and open decisions

| Risk | Likelihood / impact | Mitigation |
|---|---|---|
| Building our own SFU takes longer than expected | Medium / High | Phase 0 spike. Keep signaling behind an interface so LiveKit can drop in. str0m already handles ICE, DTLS, RTP, TWCC and simulcast. |
| Writing a good jitter buffer (the hardest part of "quality sound") | Medium / High | Start with a simple adaptive buffer plus Opus PLC, add WSOLA time-stretch next, and test against network emulation (`tc netem`) in CI. |
| Video rendering inside the Tauri webview (WebKitGTK) | Medium / Medium | Phase 0 spike. Fallback: native wgpu overlay surface. |
| Per-OS screen capture and hardware encoder variety | High / Medium | One `Capturer`/`Encoder` trait, openh264 software fallback, and a test matrix of real machines. |
| RxFerrix is young (v0.6, API "may change") | Medium / Medium | Pin versions, keep functions thin over our own domain modules, and contribute upstream. It's MIT-licensed, so we can fork if needed. |
| Single node limits | Low (for self-host) / Medium | Media splits out first. The DB measured about 9–18k writes/s, far above what a single community generates. |
| Ephemeral state accidentally put in the DB, making it slow | Medium / Medium | Code-review rule: anything faster than about 1 update per 10 s per user goes over the data channel. |

**Decisions to confirm before Phase 0:**

1. **Svelte 5 vs React** for the UI. I recommend Svelte 5; RxFerrix supports both equally.
2. **Our own str0m SFU vs a LiveKit sidecar.** I recommend str0m, decided by the Phase 0 spike.
3. **Target audience first:** work teams (Slack-like: threads, inbox, integrations) or communities (Discord-like: Lounges, Stages, roles). The plan supports both, but Phase 4's order depends on this.
4. **Hosting model:** self-host only, or also a hosted offering. RxFerrix isn't multi-tenant, so hosting would mean one instance per customer.

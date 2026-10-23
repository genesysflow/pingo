# Pingo backend implementation report

The backend implements all 37 public functions and the internal voice mutations in `docs/API.md`, including
space and room permissions, invites, DMs, threaded messaging, uploads, reactions, search, inbox, unread state,
typing and presence expiry, and signed media admission tokens. Tests use RxFerrix `TestApp`; media integration
tests use localhost sockets and synthetic Opus, with no external services or audio hardware.

## Running

```sh
cargo run -p pingo-server -- serve --dev --http 127.0.0.1:8090 --codegen-out apps/web/src/lib/rxferrix
```

The UI codegen path above is also the default for `--dev` without `--codegen-out` or `RXFERRIX_CODEGEN_OUT`.
Set `PINGO_WEB_DIR` to serve a built UI; `PINGO_APP_URL` sets the auth redirect origin (default
`http://localhost:5173`). Password, magic-link, GitHub, Google and Discord auth use the reference example's
configuration. OAuth providers without configured credentials are skipped. The dev mailer logs emails and
exposes the most recent code through `devApi.dev.last_code`.

The same binary now serves the RxFerrix API and `GET /rtc` WebSocket signaling on the HTTP(S) origin.
`cargo run -p pingo-server -- serve --dev` works unchanged. Media uses a separate UDP socket:

```sh
PINGO_UDP_PORT=7882 PINGO_PUBLIC_IPS=203.0.113.10 \
  cargo run -p pingo-server -- serve --http 0.0.0.0:7881
```

- `PINGO_UDP_PORT`: UDP bind port, default **7882**; `0` selects an ephemeral port (logged on initialization).
- `PINGO_PUBLIC_IPS`: comma-separated IP addresses advertised as ICE host candidates. Omit it to detect local
  non-loopback interfaces. IPv4 candidates bind `0.0.0.0`, IPv6 candidates bind `[::]`; all supplied addresses
  must use the same family. When testing on localhost, set `PINGO_PUBLIC_IPS=127.0.0.1`.
- Allow the selected UDP port through the host firewall and forward the same port through NAT. A reverse proxy
  must support WebSocket upgrades for `/rtc`; expose HTTPS/WSS for browser clients and forward UDP directly.
  ICE-TCP and TURN integration remain SFU limitations, so UDP connectivity is currently required.

The media secret is an **RxFerrix Environment variable**, read through `ctx.env`, rather than an OS environment
lookup. Set it in the dashboard's Environment page while serving, or configure it offline with:

```sh
cargo run -p pingo-server -- env set PINGO_MEDIA_SECRET
```

This command reads its value from stdin. With a custom data directory, pass the same `--dir` to the environment
command and server. The embedded SFU reads the current database value for **each Join**, with a two-second
lookup deadline, outside its media loop. No secret cache or second OS secret is used. Dashboard changes take
effect at the next admission lookup: tokens issued with the previous key are rejected, and fresh `voice.join`
tokens work immediately. Existing connected sessions continue. Missing/empty keys or lookup failures refuse
admission in production. In `--dev` only, an absent/empty key uses `pingo-insecure-dev-media-secret` for both
signing and verification and logs a warning. Tokens expire after 60 seconds and carry independently checked
speak, video and screen-share capabilities; members with VIEW alone may listen. Send the token as the first
WebSocket JSON message (`Join`), as specified by `pingo-proto`.

RxFerrix owns the Tokio runtime and supplies its running `App` to the `/rtc` handler. A shared Tokio `OnceCell`
initializes the SFU on the **first `/rtc` request**; simultaneous requests wait for the same initialization.
Before binding UDP or admitting anyone, internal `voice:reset` deletes crash residue in transactions of at most
500 rows. Until that first request, old rows may still appear in queries. Failed initialization returns HTTP
503 and may be retried by a later request. No custom startup HTTP endpoint is exposed.

State and leave hooks call the typed internal `voice::set_state` and `voice::left` mutations. The SFU executes
them serially on its separate hook worker; the backend tries each write up to three times (100/200 ms backoff,
four-second total budget), logging failures without blocking media or spawning unordered writes. RxFerrix's
close notification requests SFU shutdown and leave-hook draining. Hook delivery remains in memory: a process
crash or exhausted write retries are repaired by the next process's initialization sweep and the existing
stale-row cron.

Seed a development server with `devApi.dev.seed` (HTTP function path `dev:seed`, empty args). It creates a demo
space, general and design chat rooms, a Lounge, three sample users, six messages including a thread, and two
inbox notifications. It is idempotent and is refused outside `--dev`. When called signed in, the caller owns
the space; an anonymous seed uses the first demo user. Demo users can sign in through the dev magic-link mailer.

## Verification

```sh
cargo test -p pingo-server -p pingo-sfu
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt -p pingo-server -p pingo-sfu --check
```

The backend suite includes in-process full-router tests with temporary data directories, dev mode, two users
signed in through `App::auth().sign_in_as`, authenticated HTTP `voice.join`/query calls, ephemeral HTTP/UDP
ports and real str0m ICE/DTLS/SRTP peers. They verify crash-residue cleanup, Opus track forwarding, participant
and room query updates, mute/deafen, disconnect and runtime-close cleanup. Additional tests rotate the database
secret twice, reject old/wrong signatures, exercise empty/removed keys and confirm production fails closed.
The SFU suite independently covers signaling, data channels, track mapping and video forwarding.

Verified on 2026-10-23: **20 server tests and 15 SFU tests pass**, workspace clippy passes with warnings denied,
and both changed crates pass formatting checks. Workspace-wide formatting still reports pre-existing differences
in `pingo-proto`, outside this integration's scope. A separate `cargo run ... serve --dev` smoke check used HTTP
magic-link sign-in, `voice.join`, a real `/rtc` upgrade and join/leave query checks. SIGTERM also closed an active
media WebSocket; read-only SQLite inspection after exit confirmed zero voice rows.

## Generated api.ts function list

```text
api.dms:       list, open
api.files:     generateUploadUrl
api.inbox:     count, list, markAllDone, markDone
api.invites:   accept, create, preview
api.messages:  edit, list, remove, search, send, thread
api.presence:  heartbeat, setStatus
api.profiles:  me, update
api.reactions: toggle
api.rooms:     create, get, list, markRead, remove, update
api.spaces:    create, get, leave, list, members, update
api.typing:    list, start
api.voice:     join, participants

devApi.dev:    last_code, seed
```

Internal mutations, including `voice:setState` and `voice:left`, are intentionally absent from client codegen.
Rust SFU integration may call `app.mutation::<voice::set_state>(...)` using its snake_case alias, and
`app.mutation::<voice::left>(...)`. RxFerrix 0.6.1 preserves function names (only argument/field names are
converted automatically), so contracted camelCase entry points use locally allowed camelCase Rust names.

## Integration notes and limits

- The contract exposes no private-room membership or role-management mutations. Private rooms initially admit
  their creator and the space owner. DMs admit exactly their two participants; public space rooms still require
  space membership. Space owners have every permission, including access to private space rooms.
- Added internal `dm_members` and `rate_limits` tables provide narrow user-scoped DM reads and fixed-window
  send/upload limits. Rooms additionally store participants and a canonical DM pair key. These are additive
  implementation details; no existing public function name, argument or return shape was changed.
- Sends are limited to 120/minute/user and upload grants to 30/minute/user. Twenty total reaction rows per
  message enforce the contract's reaction limit. Message bodies use Unicode character counts.
- Large non-paginated lists are bounded: 500 spaces/DMs, 1,000 rooms/members/thread replies/voice participants,
  50 typing users and search hits, 100 inbox items. Message history is cursor-paginated. Bulk cleanup and inbox
  completion continue in scheduled batches. Inbox/member/reaction/reply counts use live aggregates.
- Attachment URLs, ownership checks and file metadata are real; optional image dimensions are omitted because
  RxFerrix's stored-file metadata does not provide them. No image probing pipeline was added.
- Speaking, levels, pointers and other high-frequency media events remain outside the database. SFU integrations
  should refresh voice state for long-lived calls; the hourly sweep removes rows with no updates for 12 hours.
- `docs/API.md` and `pingo-proto` are unchanged. The SFU adds `MediaSecretProvider` and
  `Sfu::start_with_secret_provider` while retaining its static-secret startup API for standalone callers.
  The shared `Cargo.lock` records the server's integration/test dependencies; root workspace settings are unchanged.

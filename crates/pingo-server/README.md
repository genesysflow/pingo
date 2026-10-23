# Pingo backend implementation report

The backend implements all 37 public functions and both internal voice mutations in `docs/API.md`, including
space and room permissions, invites, DMs, threaded messaging, uploads, reactions, search, inbox, unread state,
typing and presence expiry, and signed media admission tokens. Seventeen tests use RxFerrix `TestApp` and need
no network or audio hardware.

## Running

```sh
cargo run -p pingo-server -- serve --dev --http 127.0.0.1:8090 --codegen-out apps/web/src/lib/rxferrix
```

The UI codegen path above is also the default for `--dev` without `--codegen-out` or `RXFERRIX_CODEGEN_OUT`.
Set `PINGO_WEB_DIR` to serve a built UI; `PINGO_APP_URL` sets the auth redirect origin (default
`http://localhost:5173`). Password, magic-link, GitHub, Google and Discord auth use the reference example's
configuration. OAuth providers without configured credentials are skipped. The dev mailer logs emails and
exposes the most recent code through `devApi.dev.last_code`.

The media secret is an **RxFerrix Environment variable**, read through `ctx.env`, rather than an OS environment
lookup. Configure it before starting the server using the following command, which reads its value from stdin:

```sh
cargo run -p pingo-server -- env set PINGO_MEDIA_SECRET
```

With a custom data directory, pass the same `--dir` to the environment command and server. Share this key with
the SFU. In `--dev` only, a missing key falls back to `pingo-insecure-dev-media-secret` and logs a warning.
Production refuses admission when the key is missing. Tokens expire after 60 seconds and carry independently
checked speak, video and screen-share capabilities; members with VIEW alone may listen.

Seed a development server with `devApi.dev.seed` (HTTP function path `dev:seed`, empty args). It creates a demo
space, general and design chat rooms, a Lounge, three sample users, six messages including a thread, and two
inbox notifications. It is idempotent and is refused outside `--dev`. When called signed in, the caller owns
the space; an anonymous seed uses the first demo user. Demo users can sign in through the dev magic-link mailer.

## Verification results

Verified on 2026-10-23:

- `cargo test -p pingo-server`: **17 passed**, no failures or ignored tests. Covers owner/default permissions,
  space and private-room isolation, invite expiry/use limits, DM reuse/privacy, send/edit/delete authorization,
  mentions/replies/DM inbox delivery, bulk inbox completion without consuming later arrivals, capped unreads,
  newest-first pagination, no-op read tracking, reactions, extended typing expiry, presence expiry/reconnect,
  voice token verification/capabilities, SFU state/cleanup, membership departure and send rate limiting.
- `cargo fmt -p pingo-server --check` and `git diff --check`: clean.
- `cargo clippy -p pingo-server --all-targets`: succeeds, **no warnings in pingo-server**. The existing dependency
  `pingo-proto/src/signal.rs:11` emits `clippy::doc_lazy_continuation`; that other agent's file was not edited.
- `cargo clippy -p pingo-server --all-targets --no-deps -- -D warnings`: succeeds with **no warnings**.
  Applying `-D warnings` to dependencies too fails on the pre-existing pingo-proto documentation warning above.
- `cargo run -p pingo-server -- codegen --out /tmp/pingo-codegen-check`: succeeds. Compared all generated public
  paths with `docs/API.md`: **exactly all 37 match**. Optional return fields generate optional TS properties;
  the join URL generates the literal type `"/rtc"`.
- `cargo run -p pingo-server -- functions`: confirms `voice:setState` and `voice:left` have internal visibility.
- Started `serve --dev --http 127.0.0.1:8090` against a temporary data directory, with codegen redirected to
  `/tmp/pingo-codegen-check` to avoid editing the web agent's files. Two HTTP `dev:seed` calls succeeded and
  returned the same space ID without advancing the second write timestamp. Read-only inspection confirmed
  one space, three rooms, three users/profiles/members, six messages and two notifications. Stopped the server.

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

- `mount_media` intentionally returns the app unchanged. `/rtc` must be mounted using the SFU router and
  `app_routes` during integration. Token signing and all voice-state mutations are implemented.
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
- `docs/API.md` and `pingo-proto` were unchanged. The only shared-file edit is the generated four-dependency
  addition to `Cargo.lock`; all other changes are in `crates/pingo-server`.

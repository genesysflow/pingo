# Agent guide for Pingo

Pingo is a Slack/Discord alternative: chat, voice and screen sharing on top of the RxFerrix Rust backend.
Read `PLAN.md` for the full design. `docs/API.md` is the backend↔UI contract, and `crates/pingo-proto` is the
server↔SFU↔client contract. Don't change either without updating every side that uses it.

## Layout

| Path | What | Owner task |
|---|---|---|
| `crates/pingo-proto` | Media tokens, signaling and data-channel message types | shared, already done |
| `crates/pingo-server` | RxFerrix app: tables, queries, mutations, auth; mounts the SFU at `/rtc` | backend |
| `crates/pingo-sfu` | str0m-based SFU library and standalone binary | sfu |
| `crates/pingo-media` | Native media engine for the desktop app (audio pipeline, Opus, jitter buffer, capture) | media |
| `crates/pingo-denoise` | DeepFilterNet3 noise suppression (vendored runner + model), native and WASM for the browser | media |
| `apps/web` | Svelte 5 + Vite UI (shared by web and desktop) | web |
| `apps/desktop` | Tauri 2 shell using `pingo-media` | later |

Several agents work in parallel in separate git worktrees. **Only edit files in your own area.** If you need a
change in a shared file (root `Cargo.toml`, `pingo-proto`, `docs/API.md`), keep it minimal and mention it in your
final summary.

## Toolchain

- Rust: if `cargo` fails with a mise "No version is set" error, run `export PATH="$HOME/.cargo/bin:$PATH"` first
  (rustc 1.99, edition 2024).
- JS: `bun` 1.4. No pnpm.
- WASM: browser noise suppression needs `rustup target add wasm32-unknown-unknown`; `bun run denoise` in
  `apps/web` builds it.
- System libraries available: libopus, alsa, libpipewire-0.3, webkit2gtk-4.1.
- The machine is shared by several builds at once. `.cargo/config.toml` caps cargo at 6 jobs. Don't raise it.
- `.cargo/config.toml` also sets `LIBSQLITE3_FLAGS`, which RxFerrix needs. Keep it.

## RxFerrix reference

- RxFerrix 0.9.0 from crates.io; the client packages are `@rxferrix/client` and `@rxferrix/svelte` 0.9.0.
- Source checkout for reading: `/tmp/rxf-src/rxferrix`. If it's missing, run
  `git clone --depth 1 https://github.com/genesysflow/rxferrix /tmp/rxf-src/rxferrix`.
- What to read there:
  - `docs/CONTRACT.md` for the full Rust API.
  - `docs/guide/*` for guides.
  - `docs/guide/things-to-know.md` for caveats.
  - `examples/chat-react/server/src/*.rs` and `examples/chat-react/web/src`: the closest existing app to ours.
    Copy its patterns.
  - `packages/svelte` for the Svelte bindings.
- Key rules:
  - Fast-changing state goes in small tables of its own, with narrow index reads and no no-op writes.
  - Queries must be deterministic: use `ctx.now()`, never `SystemTime`.
  - No network I/O in mutations.
  - Anything that changes several times per second (speaking, levels, pointers) never goes in the DB. It uses
    the SFU data channel.
- Custom HTTP/WebSocket routes go through `RxFerrix::app_routes(axum::Router<rxferrix::App>)`. They must not
  overlap `/api/*`, `/_/*` or `/metrics`.

## Conventions

- Rust:
  - Run `cargo fmt` and `cargo clippy --all-targets` and leave no warnings in your crate.
  - Use `thiserror` for library errors and `tracing` for logs.
  - Doc comments explain *why*, briefly.
- Tests:
  - Every crate has meaningful tests that run with `cargo test -p <crate>` and need no network or audio
    hardware. Gate hardware-dependent code behind cargo features or `#[ignore]`.
  - Backend tests use `rxferrix::testing::TestApp`.
- Commits: work on your branch and commit in logical steps with clear messages. Don't push.
- Finish with a short report:
  - what works, and how it was verified (the commands run and their results);
  - what's stubbed or left out;
  - any contract changes.

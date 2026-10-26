# Pingo

Chat, voice and screen sharing for gaming groups, friends, communities and teams, on a server you run yourself.

- Rooms, threads, reactions, DMs, an inbox for mentions and replies, search.
- Calls with Opus at 48 kHz, DeepFilterNet3 noise suppression, and a stereo Studio mode for music.
- Screen sharing with sharp text up to 4K or smooth 1080p60, with pointers and drawing.
- A web app and a desktop app (Tauri) with a native audio engine.

## Run it

Download the server setup bundle from the website's self-hosting page and extract it into an empty directory.
It uses a prebuilt container: no Git checkout, Rust or Bun required. From that directory:

```sh
cp .env.example .env   # set your URL, public IP, media secret and domain
docker compose --profile https pull
docker compose --profile https up -d
```

The [self-hosting guide](docs/SELF_HOSTING.md#quick-start) also links the individual setup files.
The source checkout's root Compose file remains available for building locally.

See [docs/SELF_HOSTING.md](docs/SELF_HOSTING.md) for settings, invite-only registration, email allowlists,
networking for calls, and backups.

## Presentation & docs site

[apps/site](apps/site/README.md) builds the product overview and browsable documentation for GitHub Pages.
It includes a deployment workflow and renders the existing guides directly from Markdown.

## Develop

[PLAN.md](PLAN.md) has the design, [AGENTS.md](AGENTS.md) the layout and toolchain, and
[apps/web/README.md](apps/web/README.md) the web app.

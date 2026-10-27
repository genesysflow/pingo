# Meet Pingo

Pingo brings chat, voice and screen sharing together for gaming groups, friends, communities and teams, on a server you run yourself.

Use it in a browser, or build the Tauri desktop app with its native Rust audio engine. The server bundles the web app, authentication, chat and media relay into one container, with an embedded SQLite database and a persistent data directory.

## A space for your people

Create a space, make rooms for your games, interests or projects and invite your people. Each room has a conversation and an available call. You can join a call and keep reading other rooms without losing the call controls.

- **Keep conversations together.** Messages, threads, reactions, attachments and direct messages.
- **Catch up on what matters.** An inbox for mentions, replies and DMs, plus search and unread state.
- **Talk in the same room.** Voice calls with mute, deafen, device selection and a microphone sound check.
- **Share the moment.** Browser screen sharing with Text and Motion modes, optional system audio, pointers and drawing.

## Game nights, communities and teams

- **For your gaming group.** Make rooms for the games you play, plan the next session in chat and join voice when everyone is ready. Use screen sharing to show a game or help a friend through a tricky bit. Push-to-talk and noise-suppression settings let you choose how your microphone behaves.
- **For your community.** Give shared interests a room, keep longer conversations in threads and drop into a call to hang out. Invites, roles and permissions help you decide who belongs and what they can do.
- **For your team.** Keep project discussions, files and feedback together. Move from a thread into a call, share a design or code, and catch up on mentions when you return.

These use the same rooms, calls and sharing tools. Game capture and system audio depend on your browser, platform and selected source; see the [web client guide](../../../web/README.md#calls) for support and limits.

## Voice and Studio

Voice mode uses full-band Opus at 48 kHz and DeepFilterNet3 noise suppression. Choose Off, Gentle or Strong suppression in the audio settings. The browser loads the noise-suppression module when needed; the desktop client runs the model natively.

Studio mode requests stereo audio and disables microphone processing, including echo cancellation and noise suppression. It is intended for music and other situations where you want to preserve the source. Use headphones to avoid feedback.

Screen resolution, frame rate and system-audio capture depend on the browser, device and source. Text mode requests detail up to 4K; Motion requests 1080p60. These are capture targets, not performance guarantees.

## Your server, your membership

Choose open registration, invite-only access, an email allowlist, or combine invites and an allowlist. Pingo includes password sign-in and can use magic links and configured OAuth providers. Email confirmation and magic links need SMTP.

The [self-hosting guide](../../../../docs/SELF_HOSTING.md) covers Docker Compose, HTTPS, networking and the optional TURN relay, access policies, backups and upgrades.

## Current boundaries

Pingo is in active development. Calls prefer direct UDP to the server's media port. Where a network blocks it, an optional [TURN relay](../../../../docs/SELF_HOSTING.md#optional-turn-relay) can carry calls over UDP, TCP or TLS; you run and configure that relay yourself. There is no ICE-TCP fallback without TURN. Chat works either way.

Chromium has automated browser coverage. Safari, Firefox, physical audio devices and wide-area networking need further verification. Desktop packaging is verified on Linux; macOS and Windows remain unverified. Desktop OAuth browser return and automatic updates are not implemented. See the [web](../../../web/README.md) and [desktop](../../../desktop/README.md) guides for the detailed limits of each client.

## Where to go next

- [Self-host Pingo](../../../../docs/SELF_HOSTING.md) to bring up a server.
- [Understand the architecture](architecture.md) to see how chat and media fit together.
- [Develop the web app](../../../web/README.md) or [desktop app](../../../desktop/README.md).
- [Read the backend API contract](../../../../docs/API.md) before changing an integration.

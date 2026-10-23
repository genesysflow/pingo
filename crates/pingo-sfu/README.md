# pingo-sfu

An embedded or standalone str0m 0.24 SFU. The backend can merge
`sfu.router::<rxferrix::App>()` through `RxFerrix::app_routes`; the library serves
only `GET /rtc`. `Sfu::udp_addr()` exposes ephemeral bindings and
`Sfu::shutdown().await` closes sessions and drains voice hooks.

## How it runs

One Tokio task owns the shared UDP socket, rooms and every participant's `Rtc`.
UDP is demultiplexed with `Rtc::accepts`. WebSocket tasks use bounded queues;
slow signaling consumers disconnect, congested UDP sends and ephemeral data
messages are dropped. Hooks run in order on a separate task, so a delayed backend
cannot block media or reorder an old session's leave after its replacement's
join. Each hook has a five-second timeout (logged, no automatic retry); the
backend should retain its stale voice-state sweep.

Tokens are verified with `pingo_proto::token::verify`. A user has one session
across this SFU instance; replacing it closes the old socket and emits leave
before join. Sessions expire after 30 seconds without WebSocket or accepted UDP
traffic. The initial Join has a ten-second deadline, as do answers to server
offers. `Ping` receives `Pong`.

`PublishTracks` is the complete current snapshot of the client's sending mids;
send it immediately after each client offer, including an empty list to
unpublish everything. Unknown, duplicate or media-kind-mismatched mids are
rejected. Capability-denied tracks are ignored. Source tracks are never inferred
from m-line order. A subscriber receives its own negotiated mid in
`TrackPublished`, after its answer, and `TrackUnpublished` when a source stops.
A winning client offer rolls back an outstanding server offer; pending
subscriptions are retried. Receivers must support the source codec: this SFU
does not transcode.

Opus payloads pass through unchanged, preserving encoder FEC. RFC 2198 RED is
negotiated through str0m when both ends support it; ordinary Opus remains
compatible. RFC 6464 levels and talkspurt markers are retained. Every 200 ms,
active levels are sent as `Speaking` on `pingo`, including a zero level when a
speaker stops. In large rooms the loudest N audio publishers are forwarded with
a 6 dB incumbent advantage and 600 ms activity expiry. Small rooms forward all
unmuted audio, including comfort noise. Sources without audio-level extensions
get equal conservative priority. `max_forwarded_audio = 0` disables audio
forwarding. Muting suppresses all of a sender's audio; deafening suppresses all
of a receiver's audio.

VP8 and H.264 use per-subscription simulcast selection. `SetVideoQuality` names
the receiving mid; height zero pauses it. TWCC/REMB estimates are divided among
the subscriber's active videos with an audio reserve. RID `max-height` and
`max-br` attributes determine layer cost. Without those attributes, familiar
`q/h/f` and `l/m/h` RID names determine order; other SDP layers are assumed to
run from low to high (camera: 180/360/720/1440; screen:
360/1080/3240), with lower bitrate estimates for screen content to favor detail
over frame rate. An undersized budget falls back to the lowest available layer.
Layer upgrades have a one-second hold; transitions wait for a keyframe while the
old layer continues. PLI/FIR requests are translated to the origin and throttled
per source/RID; RTP timestamps are rebased across layer changes.

The data channel accepts `Reaction`, `RaiseHand`, `Pointer` and `Stroke`, stamps
the authenticated sender's participant ID and relays within its room. Clients
cannot forge `Speaking`. Messages have a 16 KiB application limit, a 60 msg/s
token bucket with a 60-message burst and bounded annotation fields. Screen mids
are translated per receiver for pointer/stroke relay. No ephemeral events touch
the database.

`stats()` reports current rooms, participants and authorized source tracks plus
cumulative accepted UDP bytes in/out (including ICE/DTLS/RTCP overhead).

## Standalone

Set `PINGO_MEDIA_SECRET` to exactly the backend's media-token secret, then:

```sh
cargo run -p pingo-sfu --bin pingo-sfu -- \
  --http 0.0.0.0:7881 --udp 0.0.0.0:7882 --public-ip YOUR_PUBLIC_IP
```

`--public-ip` can be repeated or comma-separated; when omitted, a specific UDP
bind address is advertised, or local non-loopback interfaces are detected for a
wildcard bind. Public candidates must match the socket's address family. Place
HTTP behind a TLS/WebSocket reverse proxy and expose the UDP port directly.
The standalone hooks only log state; embedded hooks should persist voice state.
Ctrl-C closes sessions and delivers their leave hooks before HTTP shuts down.

## Browser interop page

The standalone binary serves a small manual microphone/screen/data-channel page
at `/`. Paste a token from the backend, or enable local token minting:

```sh
# Use a disposable development secret, not your production secret.
PINGO_MEDIA_SECRET=local-development-only cargo run -p pingo-sfu --bin pingo-sfu -- \
  --http 127.0.0.1:7881 --udp 127.0.0.1:7882 --dev-token
```

Open `http://localhost:7881` in two Chromium tabs. Give them different users and
the same room, get dev tokens, and join. Use headphones, then test reactions or
screen sharing. The dev endpoint is `GET /dev-token?room=demo&user=alice`; it is
absent unless `--dev-token` is passed, and that flag requires a loopback HTTP
bind. Tokens expire after 60 seconds, so mint a fresh one before reconnecting.
For Firefox or other machines, use real interface/public candidates and HTTPS;
Firefox may reject loopback ICE candidates. Browser hardware/codec decoding
interop remains a manual check; automated video tests use synthetic codec
headers rather than a decoder.

## Verification and remaining work

```sh
cargo test -p pingo-sfu
cargo clippy -p pingo-sfu --all-targets -- -D warnings
cargo fmt -p pingo-sfu --check
cargo test -p pingo-proto
```

Unit tests cover room cleanup/replacement, token rejection, capability and track
validation, offer collision/stop negotiation, inactivity, speaker hysteresis,
annotation validation/stamping/rate limits, layer selection and RTP clock
continuity. Localhost integration tests drive real WebSocket signaling and
str0m ICE/DTLS/SRTP/SCTP clients for Opus and audio levels, VP8/H.264 simulcast
quality changes and keyframe requests, data relay, replacement, hooks and token
errors. They need neither audio devices nor external services.

TODO: ICE-TCP, TURN relay deployment, publisher layer suspension (dynacast),
measured layer bitrate/resolution feedback instead of estimates when RID
attributes are absent, and load/loss/browser interoperability testing. There is
no transcoding, recording or E2EE. Hook delivery is currently a serial in-memory
queue, so a persistently slow backend requires operational monitoring; it is
not a durable outbox.

No wire contract changed. The only `pingo-proto` edit separates its signaling
flow doc list from the following paragraph for warning-free clippy. The root
lockfile records the SFU's dependencies; root workspace settings are unchanged.

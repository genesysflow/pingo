# Self-hosting Pingo

Pingo is one container: the chat and auth server, the media server for calls and screen shares, and the web app.
Everything it stores lives in one data directory (`/data` in the container).

## What you need

- A Linux x86-64 machine with Docker and Docker Compose. Git, Rust and Bun are not required.
- A domain name pointing at it, for HTTPS. Browsers only allow microphones and screen sharing on `https://` pages
  (or `localhost`).
- Open **443/tcp** for HTTPS and **40000/udp** for voice and video. The included Caddy proxy also needs **80/tcp** for certificate issuance and HTTP redirects.
- Optional: an SMTP account, for magic-link sign-in and email confirmation.

## Quick start

Install [Docker](https://docs.docker.com/engine/install/) with the `docker compose` command available on your server first.
Then run the setup script from the Pingo website:

```sh
curl -fsSL https://raw.githubusercontent.com/genesysflow/pingo/main/deploy/install.sh | bash
```

The command on the website uses that site's actual address. The script asks for your domain and public IP,
generates a random media secret, writes the configuration to `./pingo`, and starts Pingo and Caddy with Docker
Compose. Caddy handles HTTPS. Point your domain at the server and open the ports listed above before starting.
The first account can create the first space; everyone after that needs an invite.

You can also [save the script](../deploy/install.sh), read it, and run it with explicit settings:

```sh
bash install.sh --domain chat.example.com --public-ip 203.0.113.10
```

Use `--dir /opt/pingo` to choose the install directory, `--no-https` if you already have an HTTPS reverse proxy,
or `--no-start` to write and validate the files without starting the services. The script refuses to overwrite
existing configuration. Run `bash install.sh --help` for all options.

The script uses `ghcr.io/genesysflow/pingo:latest` by default. Pass `--image` or set `PINGO_IMAGE` to use
a different published version or registry.

## Docker Compose

If you prefer to configure Docker Compose yourself, create a `compose.yaml` with the following content.
Replace the image address, domain and public IP with your own. Generate the media secret with
`openssl rand -hex 32` and put it in `PINGO_MEDIA_SECRET`.

```yaml
services:
  pingo:
    image: ghcr.io/genesysflow/pingo:latest
    restart: unless-stopped
    environment:
      RXFERRIX_PUBLIC_URL: https://chat.example.com
      PINGO_PUBLIC_IPS: 203.0.113.10
      PINGO_MEDIA_SECRET: REPLACE_WITH_YOUR_RANDOM_SECRET
      PINGO_REGISTRATION: invite
      RXFERRIX_TRUST_PROXY: "true"
    ports:
      - "8090:8090/tcp"
      - "40000:40000/udp"
    volumes:
      - pingo-data:/data

volumes:
  pingo-data:
```

Then start it:

```sh
docker compose pull
docker compose up -d
```

This exposes Pingo on port 8090. Put your HTTPS reverse proxy in front of it, passing WebSockets through
(`/api` and `/rtc` use them). Caddy, Traefik and nginx with the usual `Upgrade` headers all work.
Audio and video also need direct access to UDP port 40000.

For the ready-made setup with automatic HTTPS, save the [standalone Compose file](../deploy/compose.yaml),
[Caddyfile](../deploy/Caddyfile) and [environment template](../.env.example) in one directory. Copy the template
to `.env`, edit your settings, then run `docker compose --profile https up -d`.

Open `PINGO_URL`, create an account and create your first space. Invite people from
**Space settings & invites**.

The first start also prints a one-time link to create a dashboard administrator:

```sh
docker compose logs pingo | grep -A2 "No administrator"
# or: docker compose exec pingo pingo-server admin create you@example.com
```

The dashboard (`/_/`) shows data, logs and settings, and lets you change Environment variables at run time.

## Settings

The setup script puts settings in `.env`. With the single-file Compose example, edit the service’s
`environment` block instead. The image address is `PINGO_IMAGE` in the script setup or `image` in Compose.

| Variable | Required | What it does |
|---|---|---|
| `PINGO_IMAGE` | yes, prebuilt setup | Published container image, e.g. `ghcr.io/genesysflow/pingo:latest`. Use a published version tag or digest to pin upgrades. |
| `PINGO_URL` | yes | The address people open, e.g. `https://chat.example.com`. Used for sign-in redirects and links in emails. |
| `PINGO_PUBLIC_IPS` | yes | The public IP address(es) browsers can reach this server on, comma-separated. Calls send audio and video straight to `PINGO_PUBLIC_IPS:PINGO_UDP_PORT`. |
| `PINGO_MEDIA_SECRET` | yes | A long random string that signs call tokens: `openssl rand -hex 32`. |
| `PINGO_REGISTRATION` | | `open` (default) or `invite`. See below. |
| `PINGO_ALLOWED_EMAILS` | | Only these addresses and domains may use the server. See below. |
| `PINGO_DOMAIN` | with `https` | The domain Caddy gets a certificate for. |
| `PINGO_UDP_PORT` | | The media port, default `40000`. Open it for UDP in your firewall. |
| `PINGO_HTTP_PORT` | | Host port for HTTP, default `8090`. |
| `RXFERRIX_SMTP_URL`, `RXFERRIX_SMTP_FROM` | | Email for magic links and confirmation codes, e.g. `smtps://user:pass@smtp.example.com:465`. |
| `RXFERRIX_TRUST_PROXY` | | `true` behind a reverse proxy, so sign-in rate limits see real client IPs. |
| `RXFERRIX_AUTH_GITHUB_CLIENT_ID` / `_SECRET` | | Sign in with GitHub. `GOOGLE` and `DISCORD` work the same way. The callback URL is `PINGO_URL/api/auth/oauth/<provider>/callback`. |

## Who can join

By default anyone who can reach the server can create an account. Two settings narrow that down, and you can
combine them.

### Invite only

```sh
PINGO_REGISTRATION=invite
```

New accounts can only be created through a valid invite link; password, magic-link and OAuth sign-ups without
one are rejected before an account is created. On a brand-new server, the first account is let in so it can
create the first space; after that, everyone needs an invite. Existing accounts without space membership see
a "You need an invite" screen where they can paste a link.

People who already belong to a space keep their access when you switch an existing server to invite-only. If
someone leaves every space, they need a new invite.

### Only certain email addresses

```sh
PINGO_ALLOWED_EMAILS=ada@example.com, grace@example.com, @mycompany.com
```

Entries are full addresses or whole domains (`@mycompany.com` or `mycompany.com`; subdomains are not included).
Other addresses are rejected before an account is created, and invites don't override the list.

The address must be confirmed: password sign-ups get a code by email, so **set up SMTP** when you use this.
GitHub, Google and Discord sign-ins, and magic links, confirm the address on their own.

### Changing the policy without a restart

Both settings can also be set as Environment variables in the dashboard (`/_/`, **Environment**), or offline with
`docker compose exec pingo pingo-server env set PINGO_REGISTRATION invite`. Values there take precedence over
`.env`, and apply immediately: everyone connected is re-checked. To stop all new accounts, including ones from
invites, turn off **Sign-ups** in the dashboard's **Settings**.

A typo in `.env` (say `PINGO_REGISTRATION=closed`) stops the container at startup instead of leaving the server
open. A typo in the dashboard locks everyone out with "Registration is misconfigured" until you fix it.

## Calls behind NAT and firewalls

Calls need UDP between browsers and `PINGO_PUBLIC_IPS:PINGO_UDP_PORT`:

- **Cloud server:** set `PINGO_PUBLIC_IPS` to its public IP and allow `40000/udp` in the provider's firewall.
- **Home server:** forward `40000/udp` on your router to the machine, and use your public IP. Add the LAN
  address too (`PINGO_PUBLIC_IPS=203.0.113.10,192.168.1.20`) so people at home connect directly.
- **Tailscale only:** use the machine's Tailscale IP and serve HTTPS with `tailscale serve --https=443
  http://127.0.0.1:8090`.
- **Linux, simplest networking:** add `network_mode: host` to the `pingo` service and drop its `ports:`. Pingo
  then advertises every interface address itself, and `PINGO_PUBLIC_IPS` becomes optional.

### Optional TURN relay

TURN lets browsers and the native desktop engine reach the SFU when direct client UDP is blocked.
Direct UDP remains preferred. TURN carries the same encrypted media and data channels; it does not replace
`/rtc` signaling or the SFU. The relay must be able to send UDP to the SFU's advertised media endpoint.

Pingo supports coturn REST authentication. Set **both** values in `.env` or the dashboard Environment:

```sh
PINGO_TURN_URLS=turn:turn.example.com:3478?transport=udp,turn:turn.example.com:3478?transport=tcp,turns:turn.example.com:5349?transport=tcp
PINGO_TURN_SECRET=REPLACE_WITH_A_SEPARATE_RANDOM_SECRET
```

Generate a separate secret with `openssl rand -hex 32`, and configure coturn with the same
`static-auth-secret` and `use-auth-secret`. Never use `PINGO_MEDIA_SECRET` for this. Pingo returns temporary
credentials only after checking room access. They last one hour; clients automatically rejoin with fresh
credentials five minutes before expiry, briefly interrupting media. Browser screen/camera capture is retained
across that rejoin. Clear both settings to disable TURN. Partial or invalid configuration rejects call admission
with a configuration error. The supported URL schemes are `turn:` and `turns:`, with at most eight URLs.

The setup script accepts `--turn-urls 'turn:turn.example.com:3478?transport=tcp'`. It generates a separate
TURN secret in `.env`; configure your coturn with that secret. It also writes the optional coturn sidecar files.
No TURN process starts until you explicitly enable the `turn` Compose profile.

For the included **Linux** sidecar, configure:

```sh
# Alongside PINGO_TURN_URLS and PINGO_TURN_SECRET above:
PINGO_TURN_RELAY_IP=192.0.2.10
PINGO_TURN_PUBLIC_IP=203.0.113.10
PINGO_TURN_PEER_IPS=203.0.113.10
# Optional TURN/TLS listener; omit both TLS settings for UDP/TCP only:
PINGO_TURN_TLS_PORT=5349
PINGO_TURN_CERTS_DIR=./turn-certs
```

`PINGO_TURN_RELAY_IP` must be assigned to the relay host. When behind NAT, use its private IP here and the
public IP in `PINGO_TURN_PUBLIC_IP`. `PINGO_TURN_PEER_IPS` allows only the comma-separated SFU candidate
IPs as relay destinations; all other destinations are denied. For colocated hosts behind NAT without hairpin
routing, include a reachable LAN candidate in `PINGO_PUBLIC_IPS` and allow that IP too. An existing coturn
installation needs equivalent destination restrictions and quotas.

Save [turn-entrypoint.sh](../deploy/turn-entrypoint.sh) beside the standalone `compose.yaml` (the source
checkout already has it under `deploy/`), then run:

```sh
docker compose --profile https --profile turn up -d
```

The sidecar uses host networking to avoid thousands of Docker port-forwarding rules. Allow **3478/udp**,
**3478/tcp**, **5349/tcp** if TLS is enabled, and **49160–49200/udp** for relay allocations. Forward those
ports when behind NAT. Ports and relay range can be overridden with `PINGO_TURN_PORT`,
`PINGO_TURN_TLS_PORT`, `PINGO_TURN_MIN_PORT`, and `PINGO_TURN_MAX_PORT`.

TURN/TLS requires a trusted certificate: mount `fullchain.pem` and `privkey.pem` in the certificate directory.
The native engine trusts WebPKI and system roots; browsers use their own certificate trust. Provision and renew
these certificates using your certificate manager, then restart coturn after renewal. Caddy's automatic HTTPS
certificate storage is not automatically shared with coturn.

For networks allowing only port 443, use `turns:turn.example.com:443?transport=tcp` with the TLS listener on
443 on a **separate host or public IP**. The included Caddy service already binds port 443 on all host IPs;
using a separate IP on the same host also requires narrowing Caddy's port bindings. An HTTP reverse proxy
cannot forward TURN. TURN/TCP and TURN/TLS describe the client-to-relay connection; relay-to-SFU traffic
still uses UDP. Explicit HTTP-proxy-only networks can still block TURN/TLS.

The defaults limit each credential to eight allocations, the relay to 200 allocations, each allocation to
2,000,000 bytes/s and aggregate allocated bandwidth to 100,000,000 bytes/s. Size the relay range, quotas
and egress budget for your expected calls. The configured range provides 41 relay ports per relay IP.

To verify a deployment, set `localStorage.setItem('pingo-relay-only', 'true')` in the web/desktop developer
console and rejoin. Browser WebRTC diagnostics should show a selected `relay` candidate. Remove the key
afterward. For local automated verification, `deploy/test-turn.sh --browser` runs real coturn UDP/TCP/TLS
and the browser call test without public services or audio hardware; Docker, OpenSSL, Python, Rust, Bun and
the existing Playwright browser installation are required.

## Backups and upgrades

```sh
# A consistent backup while running (uploaded files included):
docker compose exec pingo pingo-server backup --dir /data --with-files --out /data/backup.tar.zst
docker compose cp pingo:/data/backup.tar.zst .

# Upgrade:
docker compose --profile https pull
docker compose --profile https up -d
```

Run these commands from the install directory. Omit `--profile https` if you use your own reverse proxy.
Change `PINGO_IMAGE` (or `image` in your Compose file) first to move between pinned
versions, and back up before upgrading. Keep the same directory or Compose project name so Docker reuses your
existing data volume.

To restore, stop the container and run the restore against the data volume:

```sh
chmod a+r backup.tar.zst   # the container runs as an unprivileged user
docker compose stop pingo
docker compose run --rm -v "$PWD/backup.tar.zst:/backup.tar.zst:ro" pingo restore --dir /data /backup.tar.zst
docker compose start pingo
```

See `pingo-server --help` for scheduled backups to S3, export, import and migrations.

## Build from source

For development or unsupported image architectures, use a source checkout and the root `compose.yaml`:

```sh
cp .env.example .env
$EDITOR .env
docker compose --profile https up -d --build
```

This is the optional build path; the setup script and standalone Compose file use a prebuilt image without a build context.

## Without Docker

The manual build below also requires a source checkout, Rust, Bun and the system dependencies described in the
repository's agent guide.

```sh
rustup target add wasm32-unknown-unknown
cargo build --release -p pingo-server
(cd apps/web && bun install && bun run build)
PINGO_WEB_DIR=apps/web/dist PINGO_PUBLIC_IPS=… PINGO_MEDIA_SECRET=… RXFERRIX_PUBLIC_URL=https://… \
  target/release/pingo-server serve --http 0.0.0.0:8090 --dir /var/lib/pingo
```

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

Install [Docker Engine with the Compose plugin](https://docs.docker.com/engine/install/) on your server first.
Then run the setup script from the Pingo website:

```sh
curl -fsSL https://YOUR-PINGO-SITE/install.sh | bash
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

The script needs access to Docker and a published Pingo image. The published site sets the image address;
for a local preview or another registry, pass `--image ghcr.io/OWNER/REPOSITORY:latest` or set `PINGO_IMAGE`.

## Docker Compose

If you prefer to configure Docker Compose yourself, create a `compose.yaml` with the following content.
Replace the image address, domain and public IP with your own. Generate the media secret with
`openssl rand -hex 32` and put it in `PINGO_MEDIA_SECRET`.

```yaml
services:
  pingo:
    image: ghcr.io/OWNER/REPOSITORY:latest
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
| `PINGO_IMAGE` | yes, prebuilt setup | Published container image, e.g. `ghcr.io/OWNER/REPOSITORY:latest`. Use a published version tag or digest to pin upgrades. |
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

New accounts can only get in through an invite link. The sign-up page tells visitors so, and an account created
any other way sees a "You need an invite" screen where it can paste a link. On a brand-new server, the first
account is let in so it can create the first space; after that, everyone needs an invite.

People who already belong to a space keep their access when you switch an existing server to invite-only. If
someone leaves every space, they need a new invite.

### Only certain email addresses

```sh
PINGO_ALLOWED_EMAILS=ada@example.com, grace@example.com, @mycompany.com
```

Entries are full addresses or whole domains (`@mycompany.com` or `mycompany.com`; subdomains are not included).
Anyone else who signs up sees "This address can't join", and invites don't override the list.

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

Pingo has no TURN relay yet, so networks that block outgoing UDP (some corporate networks) can't join calls.
Chat still works there.

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

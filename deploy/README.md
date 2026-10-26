# Pingo server setup

Run the setup script from the Pingo website on your Linux x86-64 server:

```sh
curl -fsSL https://YOUR-PINGO-SITE/install.sh | bash
```

It asks for your domain and public IP, generates the media secret, writes `.env`, `compose.yaml` and `Caddyfile`
to `./pingo`, then pulls and starts the containers. Install Docker with its Compose plugin first. Git, Rust
and Bun are not needed on the server. The website fills in the command's address and the published image.

To run a saved script with explicit settings:

```sh
bash install.sh --domain chat.example.com --public-ip 203.0.113.10
```

Use `--dir` for a different directory, `--no-https` for your own HTTPS proxy, or `--no-start` to generate the
configuration without starting services. Set `PINGO_IMAGE` or pass `--image` to select another image.
Existing `.env`, Compose and Caddy files are never overwritten. Invite-only registration is enabled; the first
account can create the first space.

The site's self-hosting guide also includes a copy-and-paste `compose.yaml` for manual setup. For the full
Caddy setup without the script, save `compose.yaml`, `Caddyfile` and `.env.example` from the site, copy the
template to `.env`, edit it, then run `docker compose --profile https up -d`.

Point the domain at your server and allow **80/tcp**, **443/tcp** and **40000/udp**. With another HTTPS proxy,
forward it to port 8090 (including WebSockets) and omit `--profile https` when running Compose commands.

From the install directory, update with:

```sh
docker compose --profile https pull
docker compose --profile https up -d
```

Pin the image to a published version or digest to control upgrades. Back up before upgrading:

```sh
docker compose exec pingo pingo-server backup --dir /data --with-files --out /data/backup.tar.zst
docker compose cp pingo:/data/backup.tar.zst .
```

## Publishing the container

For maintainers: `.github/workflows/container.yml` builds the root Dockerfile and publishes
`ghcr.io/<owner>/<repository>:latest` on `main`. A `v0.1.0` tag publishes `:0.1.0`; every build also gets a
commit tag. The workflow currently targets `linux/amd64`.

After the first successful publish, open the container package's settings on GitHub and set its visibility to
**Public** so self-hosters can pull without signing in. A private package requires `docker login ghcr.io` with
an account allowed to read it. Repository visibility alone does not make a new GHCR package public.

The site infers the image address from `GITHUB_REPOSITORY` in Actions. A local site build can set
`SITE_REPOSITORY_URL=https://github.com/OWNER/REPOSITORY`, or use `SITE_IMAGE=registry.example.com/pingo:VERSION`
to distribute a different published image. The image must exist and be accessible before users can start Pingo.

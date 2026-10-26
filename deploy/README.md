# Pingo server setup

This bundle runs Pingo from a prebuilt container image. It needs Docker and Docker Compose on a Linux x86-64
server; Git, Rust and Bun are not needed on that server.

1. Extract the bundle into an empty directory and run commands from there.
2. Copy `.env.example` to `.env` and edit it. Set your URL, public IP, domain and a random media secret
   (`openssl rand -hex 32`). The image address is filled in by the published site; if it still contains
   `OWNER/REPOSITORY`, replace it with your published image address.
3. Run `docker compose --profile https pull`, then `docker compose --profile https up -d`.
4. Open your configured URL, create an account and invite your people.

Point the domain's DNS at your server. For the included Caddy HTTPS proxy, allow incoming **80/tcp** and
**443/tcp**; voice and video also need **40000/udp** (or your configured `PINGO_UDP_PORT`).

If you already have an HTTPS reverse proxy, omit `--profile https` and proxy it to Pingo's HTTP port
(default `8090`). Pass WebSocket connections through and keep `RXFERRIX_TRUST_PROXY=true`.

Update with `docker compose --profile https pull` followed by `docker compose --profile https up -d`.
Pin `PINGO_IMAGE` to a published version or digest if you want to control when you upgrade.

Before upgrading, make a backup:

```sh
docker compose exec pingo pingo-server backup --dir /data --with-files --out /data/backup.tar.zst
docker compose cp pingo:/data/backup.tar.zst .
```

Read the site's self-hosting guide for access policies, email, networking, backup restoration and other settings.

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

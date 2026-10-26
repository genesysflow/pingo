# Pingo presentation and documentation

A static product page and documentation site for GitHub Pages. It builds independently of the application:
no Rust, WASM, backend or authentication is needed. Bun and `marked` are the only build requirements.
Fonts and interface screenshots come from `apps/web`; screenshot content is an example workspace.
The self-hosting guide includes a one-command Docker setup script and a copy-and-paste Compose example.

## Preview locally

```sh
cd apps/site
bun install --frozen-lockfile
bun run dev
# http://localhost:4321
```

The preview serves `dist`, including real 404 responses. Re-run the build after editing files and refresh the
browser. Set `PORT` to change the preview port. To check a project-site prefix, use:

```sh
SITE_BASE_PATH=/pingo/ bun run dev
# http://localhost:4321/pingo/
```

## Build

```sh
cd apps/site
bun run build
# Static output: apps/site/dist
```

`SITE_BASE_PATH` defaults to `/` and must include leading and trailing slashes. The repository defaults to
`https://github.com/genesysflow/pingo`, so GitHub navigation and source links work in local builds too.
`SITE_REPOSITORY_URL` overrides it; in Actions the repository comes from `GITHUB_REPOSITORY`. Each guide also
has a downloadable Markdown source link.

## Publish on GitHub Pages

1. Open [genesysflow/pingo](https://github.com/genesysflow/pingo) on GitHub.
2. In **Settings → Pages → Build and deployment → Source**, choose **GitHub Actions**.
3. Merge this change into `main`. Changes to the site, guides or shared visual assets trigger
   `.github/workflows/pages.yml`; you can also use **Actions → Publish presentation and docs → Run workflow**
   on `main`.

The workflow builds and uploads `dist`, then deploys it using the `github-pages` environment. GitHub reports
the published URL on the deployment. Configure any custom domain in GitHub's Pages settings; the workflow
uses `configure-pages` to resolve its base path. User/organization sites use `/`; project sites use the repo
prefix. Pull requests build an artifact with a repository prefix, without publishing it.

GitHub Pages hosts this static presentation and documentation. The Pingo chat application requires its own
server; follow the [self-hosting guide](../../docs/SELF_HOSTING.md).

## Server setup script

Each build publishes a self-contained `install.sh` that prompts for a domain and public IP, generates a random
media secret and Docker configuration, and starts Pingo and Caddy. Its source is `deploy/install.sh`.
The self-hosting page fills in the install command using the current site URL, including project prefixes
and custom domains. It also includes a complete Compose example for manual setup.

The default image is `ghcr.io/genesysflow/pingo:latest`. A repository override through `GITHUB_REPOSITORY` or
`SITE_REPOSITORY_URL` changes the inferred image address. Set `SITE_IMAGE` to select another image explicitly.
The image must be published and readable; see [container publishing](../../deploy/README.md#publishing-the-container).

The Compose file, Caddyfile and environment template remain available individually under `downloads/`.
Changes to `deploy/` and `.env.example` rebuild the Pages site. Run `bun run test` to verify the installer and
site output. Installer configuration tests use Docker Compose when installed; they need no running daemon.
The Pages workflow runs the tests before deployment.

## Maintain the content

- `src/templates.ts`: landing page, shared header/footer and docs layout.
- `src/style.css`, `src/site.js`: responsive themes, screenshot switcher, docs menu and code copying.
- `src/pages.ts`: guide titles, source files and navigation order.
- `src/content/`: introductory and architecture guides written for the site.
- Existing `docs/*.md`, app and crate READMEs: rendered directly at build time, without a second copy to edit.

Relative Markdown links to included guides are rewritten to their static routes. Other linked files are
copied to `sources/`; missing files fail the build. Only declared assets and linked documentation files are
published. The root `.nojekyll` file keeps static hosting independent of Jekyll. Directory index pages allow
direct links and refreshes without an SPA fallback.

The page uses local fonts and images and makes no third-party runtime requests. The light/dark preference
stays in the visitor's browser. Docs remain readable and navigation works with JavaScript disabled.

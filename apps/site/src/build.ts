import { cp, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { Marked, Renderer } from "marked";
import { escape, pages } from "./pages";
import { templates } from "./templates";

const root = resolve(import.meta.dir, "../../..");
const output = resolve(import.meta.dir, "../dist");
const rawBase = process.env.SITE_BASE_PATH || "/";
if (!/^\/(?:[a-zA-Z0-9_.-]+\/)*$/.test(rawBase) || rawBase.includes("/../")) {
  throw new Error(
    "SITE_BASE_PATH must be / or a path with leading and trailing slashes, e.g. /pingo/.",
  );
}
const repository =
  process.env.SITE_REPOSITORY_URL ||
  (process.env.GITHUB_REPOSITORY
    ? `https://github.com/${process.env.GITHUB_REPOSITORY}`
    : "https://github.com/genesysflow/pingo");
if (
  repository &&
  !/^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/?$/.test(repository)
) {
  throw new Error(
    "SITE_REPOSITORY_URL must be an HTTPS GitHub repository URL.",
  );
}
const base = rawBase;
const image =
  process.env.SITE_IMAGE ||
  (repository
    ? `ghcr.io/${new URL(repository).pathname.slice(1).replace(/\/$/, "").toLowerCase()}:latest`
    : "ghcr.io/genesysflow/pingo:latest");
if (!/^[a-zA-Z0-9][a-zA-Z0-9._/:@-]*$/.test(image)) {
  throw new Error(
    "SITE_IMAGE must be a container image reference without whitespace.",
  );
}
const view = templates(base, repository.replace(/\/$/, ""));
const pageBySource = new Map(pages.map((p) => [p.source, p.slug]));
const downloadBySource = new Map([
  ["deploy/compose.yaml", "downloads/compose.yaml"],
  ["deploy/Caddyfile", "downloads/Caddyfile"],
  ["deploy/turn-entrypoint.sh", "downloads/turn-entrypoint.sh"],
  ["deploy/install.sh", "install.sh"],
  [".env.example", "downloads/.env.example"],
]);

async function emit(path: string, content: string) {
  const target = resolve(output, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, content);
}

// Only selected, public documentation assets enter dist; never copy the checkout.
await rm(output, { recursive: true, force: true });
await mkdir(resolve(output, "assets/fonts"), { recursive: true });
const assets = [
  "pingo.svg",
  "fonts/instrument-sans.ttf",
  "fonts/instrument-sans-italic.ttf",
  "fonts/jetbrains-mono.ttf",
  "fonts/Instrument-Sans-OFL.txt",
  "fonts/JetBrains-Mono-OFL.txt",
];
for (const asset of assets)
  await cp(
    resolve(root, "apps/web/public", asset),
    resolve(output, "assets", asset),
  );
for (const name of ["chat-light", "chat-dark", "share-light", "share-dark"]) {
  await cp(
    resolve(root, `apps/web/screenshots/${name}.png`),
    resolve(output, `assets/${name}.png`),
  );
}
await cp(
  resolve(import.meta.dir, "style.css"),
  resolve(output, "assets/style.css"),
);
await cp(
  resolve(import.meta.dir, "site.js"),
  resolve(output, "assets/site.js"),
);

// The standalone setup has no build context and works outside a checkout.
const installer = (
  await readFile(resolve(root, "deploy/install.sh"), "utf8")
).replaceAll("ghcr.io/genesysflow/pingo:latest", image);
await emit("install.sh", installer);
const setup = {
  "compose.yaml": await readFile(resolve(root, "deploy/compose.yaml"), "utf8"),
  Caddyfile: await readFile(resolve(root, "deploy/Caddyfile"), "utf8"),
  "turn-entrypoint.sh": await readFile(resolve(root, "deploy/turn-entrypoint.sh"), "utf8"),
  ".env.example": `# Prebuilt Pingo image. Pin a published version or digest to control upgrades.\nPINGO_IMAGE=${image}\n\n${(
    await readFile(resolve(root, ".env.example"), "utf8")
  )
    .replace("# PINGO_DOMAIN=", "PINGO_DOMAIN=")
    .replace("# RXFERRIX_TRUST_PROXY=true", "RXFERRIX_TRUST_PROXY=true")}`,
  "README.md": await readFile(resolve(root, "deploy/README.md"), "utf8"),
};
for (const [name, content] of Object.entries(setup))
  await emit(`downloads/${name}`, content);

for (const [index, page] of pages.entries()) {
  const toc: { id: string; text: string }[] = [];
  const slugs = new Map<string, number>();
  const renderer = new Renderer();
  renderer.heading = function ({ tokens, depth }) {
    const text = tokens
      .map((token) => ("text" in token ? token.text : token.raw))
      .join("")
      .replace(/<[^>]*>/g, "");
    const slug = text
      .toLowerCase()
      .replace(/[^\p{L}\p{N}_\s-]/gu, "")
      .trim()
      .replace(/\s/g, "-");
    const count = slugs.get(slug) || 0;
    slugs.set(slug, count + 1);
    const id = count ? `${slug}-${count}` : slug;
    if (depth === 2) toc.push({ id, text });
    return `<h${depth} id="${escape(id)}">${this.parser.parseInline(tokens)}</h${depth}>\n`;
  };
  renderer.table = function (token) {
    return `<div class="table-scroll" tabindex="0" role="region" aria-label="Scrollable reference table">${Renderer.prototype.table.call(this, token)}</div>`;
  };
  const linkedFiles = new Set<string>();
  renderer.link = function ({ href, title, tokens }) {
    let target = href;
    if (!/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(href)) {
      const [path, hash] = href.split("#", 2);
      const resolved = resolve(
        root,
        dirname(page.source),
        decodeURIComponent(path),
      );
      const source = relative(root, resolved);
      if (source.startsWith("..") || source.includes("\\"))
        throw new Error(`Link escapes repository: ${page.source}: ${href}`);
      const slug = pageBySource.get(source);
      const download = downloadBySource.get(source);
      target = slug
        ? `${base}docs/${slug}/`
        : download
          ? `${base}${download}`
          : `${base}sources/${source}`;
      if (hash) target += `#${hash}`;
      if (!slug && !download) linkedFiles.add(source);
    }
    if (/^(?:javascript|data|vbscript):/i.test(target))
      throw new Error(`Unsafe link: ${target}`);
    return `<a href="${escape(target)}"${title ? ` title="${escape(title)}"` : ""}>${this.parser.parseInline(tokens)}</a>`;
  };
  const parser = new Marked({ gfm: true, renderer });
  const markdown = (
    await readFile(resolve(root, page.source), "utf8")
  ).replaceAll("ghcr.io/genesysflow/pingo:latest", image);
  const html = (await parser.parse(markdown)).replace(
    '<code class="language-sh">curl -fsSL https://raw.githubusercontent.com/genesysflow/pingo/main/deploy/install.sh | bash',
    `<code class="language-sh" data-install-command data-install-path="${base}install.sh">curl -fsSL https://raw.githubusercontent.com/genesysflow/pingo/main/deploy/install.sh | bash`,
  );
  await emit(`docs/${page.slug}/index.html`, view.doc(page, html, toc, index));
  await emit(`sources/${page.source}`, markdown);
  for (const source of linkedFiles) {
    const path = resolve(root, source);
    if (!(await stat(path)).isFile())
      throw new Error(`Link is not a file: ${page.source}: ${source}`);
    await mkdir(dirname(resolve(output, "sources", source)), {
      recursive: true,
    });
    await cp(path, resolve(output, "sources", source));
  }
}
await emit("index.html", view.home());
await emit("404.html", view.notFound());
await emit(".nojekyll", "");
// Directory URLs work on Pages without SPA fallbacks or server-side routing.
await emit(
  "docs/index.html",
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=${base}docs/overview/"><title>Pingo documentation</title></head><body><a href="${base}docs/overview/">Open the Pingo documentation</a></body></html>`,
);
console.log(
  `Built presentation + ${pages.length} documentation pages in apps/site/dist (base: ${base}).`,
);

import { expect, test } from "bun:test";
import { resolve } from "node:path";

const site = resolve(import.meta.dir, "..");
const root = resolve(site, "../..");
const archive = new Bun.Archive(
  await Bun.file(
    resolve(site, "dist/downloads/pingo-setup.tar.gz"),
  ).arrayBuffer(),
);
const files = await archive.files();

test("the setup archive is self-contained and uses a prebuilt image", async () => {
  expect([...files.keys()].sort()).toEqual([
    ".env.example",
    "Caddyfile",
    "README.md",
    "compose.yaml",
  ]);
  const compose = await files.get("compose.yaml")!.text();
  expect(compose).toBe(
    await Bun.file(resolve(root, "deploy/compose.yaml")).text(),
  );
  expect(compose).not.toMatch(/^\s*build:/m);
  expect(compose).toContain("image: ${PINGO_IMAGE:?");
  expect(compose).toContain("./Caddyfile:/etc/caddy/Caddyfile:ro");
  expect(await files.get("Caddyfile")!.text()).toBe(
    await Bun.file(resolve(root, "deploy/Caddyfile")).text(),
  );
});

test("the environment is ready to configure without a source checkout", async () => {
  const env = await files.get(".env.example")!.text();
  expect(env).toMatch(/^PINGO_IMAGE=\S+$/m);
  expect(env).toMatch(/^PINGO_MEDIA_SECRET=$/m);
  expect(env).toMatch(/^PINGO_DOMAIN=chat.example.com$/m);
  expect(env).toMatch(/^RXFERRIX_TRUST_PROXY=true$/m);
  if (process.env.SITE_IMAGE) {
    expect(env.split("\n")).toContain(`PINGO_IMAGE=${process.env.SITE_IMAGE}`);
  }
  // Individual downloads and the archive must contain the same settings.
  expect(env).toBe(
    await Bun.file(resolve(site, "dist/downloads/.env.example")).text(),
  );
});

test("the self-hosting guide links to a downloadable archive under the site prefix", async () => {
  const base = process.env.SITE_BASE_PATH || "/";
  const page = await Bun.file(
    resolve(site, "dist/docs/self-hosting/index.html"),
  ).text();
  expect(page).toContain(`href="${base}downloads/pingo-setup.tar.gz" download`);
  expect(page).toContain(`href="${base}downloads/compose.yaml"`);
  expect(page).toContain(`href="${base}downloads/.env.example"`);
});

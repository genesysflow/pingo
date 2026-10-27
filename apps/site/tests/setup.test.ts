import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const site = resolve(import.meta.dir, "..");
const root = resolve(site, "../..");
const script = resolve(site, "dist/install.sh");
const directories: string[] = [];
function directory() {
  const path = mkdtempSync(resolve(tmpdir(), "pingo-installer-test-"));
  directories.push(path);
  return path;
}
function configure(path: string, extra: string[] = []) {
  return Bun.spawnSync([
    "bash",
    script,
    "--dir",
    path,
    "--image",
    "pingo:test",
    "--domain",
    "chat.example.com",
    "--public-ip",
    "127.0.0.1",
    "--no-start",
    ...extra,
  ]);
}
let hasCompose = false;
try {
  hasCompose = Bun.spawnSync(["docker", "compose", "version"]).exitCode === 0;
} catch {}
afterEach(() => {
  for (const path of directories.splice(0))
    rmSync(path, { recursive: true, force: true });
});

test("the published script has valid Bash syntax and the configured image", async () => {
  const syntax = Bun.spawnSync(["bash", "-n", script]);
  expect(syntax.exitCode).toBe(0);
  const source = await Bun.file(script).text();
  if (process.env.SITE_IMAGE) expect(source).toContain(process.env.SITE_IMAGE);
  expect(source).not.toContain("git clone");
});

test.skipIf(!hasCompose)(
  "the installer generates complete, valid Docker configuration in an empty directory",
  async () => {
    const path = directory();
    const result = configure(path);
    expect(result.exitCode).toBe(0);
    const env = readFileSync(resolve(path, ".env"), "utf8");
    expect(env).toMatch(/^PINGO_MEDIA_SECRET=[a-f0-9]{64}$/m);
    expect(env).toContain("PINGO_REGISTRATION=invite");
    expect(statSync(resolve(path, ".env")).mode & 0o777).toBe(0o600);
    expect(readFileSync(resolve(path, "compose.yaml"), "utf8")).toBe(
      await Bun.file(resolve(root, "deploy/compose.yaml")).text(),
    );
    expect(readFileSync(resolve(path, "Caddyfile"), "utf8")).toBe(
      await Bun.file(resolve(root, "deploy/Caddyfile")).text(),
    );
    expect(readFileSync(resolve(path, "turn-entrypoint.sh"), "utf8")).toBe(
      await Bun.file(resolve(root, "deploy/turn-entrypoint.sh")).text(),
    );
    const config = Bun.spawnSync(
      ["docker", "compose", "--profile", "https", "config", "--format", "json"],
      { cwd: path },
    );
    expect(config.exitCode).toBe(0);
    const parsed = JSON.parse(config.stdout.toString());
    expect(parsed.services.pingo.image).toBe("pingo:test");
    expect(parsed.services.pingo.build).toBeUndefined();
    expect(parsed.services.caddy).toBeDefined();
  },
);

test.skipIf(!hasCompose)(
  "existing configuration is preserved when setup is rerun",
  () => {
    const path = directory();
    expect(configure(path).exitCode).toBe(0);
    const original = readFileSync(resolve(path, ".env"), "utf8");
    const rerun = configure(path);
    expect(rerun.exitCode).not.toBe(0);
    expect(rerun.stderr.toString()).toContain("already exists");
    expect(readFileSync(resolve(path, ".env"), "utf8")).toBe(original);
  },
);

test.skipIf(!hasCompose)(
  "setup rejects injected domain values and invalid ports",
  () => {
    const path = directory();
    expect(
      configure(path, ["--domain", "bad.example.com\nPINGO_REGISTRATION=open"])
        .exitCode,
    ).not.toBe(0);
    expect(configure(path, ["--udp-port", "70000"]).exitCode).not.toBe(0);
  },
);

test("the guide offers the script command and a manual Compose example under the site prefix", async () => {
  const base = process.env.SITE_BASE_PATH || "/";
  const page = await Bun.file(
    resolve(site, "dist/docs/self-hosting/index.html"),
  ).text();
  expect(page).toContain(`href="${base}install.sh"`);
  expect(page).toContain(`data-install-path="${base}install.sh"`);
  expect(page).toContain('id="docker-compose"');
  expect(page).toContain("REPLACE_WITH_YOUR_RANDOM_SECRET");
  expect(page).not.toContain("pingo-setup.tar.gz");
});

test.skipIf(!hasCompose)("TURN setup generates an independent secret and includes the optional relay", () => {
  const path = directory();
  const result = configure(path, ["--turn-urls", "turn:relay.example.com:3478?transport=tcp,turns:relay.example.com:443?transport=tcp"]);
  expect(result.exitCode).toBe(0);
  const env = readFileSync(resolve(path, ".env"), "utf8");
  const media = env.match(/^PINGO_MEDIA_SECRET=(.*)$/m)![1];
  const turn = env.match(/^PINGO_TURN_SECRET=(.*)$/m)![1];
  expect(turn).toMatch(/^[a-f0-9]{64}$/);
  expect(turn).not.toBe(media);
  const config = Bun.spawnSync(["docker", "compose", "--profile", "turn", "config", "--format", "json"], { cwd: path });
  expect(config.exitCode).toBe(0);
  const parsed = JSON.parse(config.stdout.toString());
  expect(parsed.services.turn.network_mode).toBe("host");
  expect(parsed.services.turn.image).toBe("coturn/coturn:4.6.3");
});

test("the published relay entrypoint is downloadable beside Compose", async () => {
  expect(await Bun.file(resolve(site, "dist/downloads/turn-entrypoint.sh")).text()).toBe(
    await Bun.file(resolve(root, "deploy/turn-entrypoint.sh")).text(),
  );
});

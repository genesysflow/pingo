/**
 * Writes the updater manifest (`latest.json`) for a release from the signed bundles `tauri build` produced
 * with `createUpdaterArtifacts`. The updater looks up `{os}-{arch}-{bundle}` first and then `{os}-{arch}`, so
 * each bundle gets its own key and the AppImage also serves as the Linux default.
 *
 *   bun scripts/update-manifest.ts --version 0.2.0 --base-url https://github.com/o/r/releases/download/v0.2.0 \
 *     --notes-file notes.md --out latest.json <bundle files...>
 */
import { readFile, writeFile } from 'node:fs/promises';
import { basename } from 'node:path';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    version: { type: 'string' },
    'base-url': { type: 'string' },
    'notes-file': { type: 'string' },
    out: { type: 'string', default: 'latest.json' },
  },
});
const version = values.version?.replace(/^v/, '');
const base = values['base-url']?.replace(/\/+$/, '');
if (!version || !base || !positionals.length) {
  console.error('usage: update-manifest.ts --version X --base-url URL [--notes-file F] [--out F] <bundles...>');
  process.exit(2);
}

/** Updater platform keys per bundle kind. Add macOS and Windows here when those builds exist. */
function keys(file: string): string[] {
  const arch = /(_amd64|_x86_64|x86_64)\b/.test(file) ? 'x86_64' : /(_arm64|aarch64)/.test(file) ? 'aarch64' : '';
  if (!arch) throw new Error(`Can't tell the architecture of ${file}`);
  if (file.endsWith('.AppImage')) return [`linux-${arch}-appimage`, `linux-${arch}`];
  if (file.endsWith('.deb')) return [`linux-${arch}-deb`];
  throw new Error(`Not an updater bundle: ${file}`);
}

const platforms: Record<string, { signature: string; url: string }> = {};
for (const file of positionals) {
  const signature = (await readFile(`${file}.sig`, 'utf8')).trim();
  const url = `${base}/${encodeURIComponent(basename(file))}`;
  for (const key of keys(file)) {
    if (platforms[key]) throw new Error(`Two bundles for ${key}`);
    platforms[key] = { signature, url };
  }
}
const notes = values['notes-file'] ? (await readFile(values['notes-file'], 'utf8')).trim() : '';
const manifest = { version, notes, pub_date: new Date().toISOString(), platforms };
await writeFile(values.out!, JSON.stringify(manifest, null, 2) + '\n');
console.log(`${values.out}: ${version} for ${Object.keys(platforms).join(', ')}`);

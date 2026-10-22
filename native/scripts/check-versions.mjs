import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptPath = fileURLToPath(import.meta.url);

export function checkVersions(nativeRoot = resolve(dirname(scriptPath), '..')) {
  const readJson = (path) => JSON.parse(readFileSync(resolve(nativeRoot, path), 'utf8'));
  const version = readJson('desktop/tauri.conf.json').version;
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error('desktop/tauri.conf.json must declare a numeric X.Y.Z version.');
  }
  for (const path of ['Cargo.toml', 'desktop/Cargo.toml']) {
    const text = readFileSync(resolve(nativeRoot, path), 'utf8');
    const section = text.match(/^\[package\]\s*\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1];
    const actual = section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
    if (actual !== version) throw new Error(`${path}: expected ${version}, got ${actual ?? 'no package version'}.`);
  }
  const manifest = readJson('web/package.json');
  const lock = readJson('web/package-lock.json');
  for (const [path, actual] of [
    ['web/package.json', manifest.version],
    ['web/package-lock.json', lock.version],
    ['web/package-lock.json packages[""]', lock.packages?.['']?.version],
  ]) {
    if (actual !== version) throw new Error(`${path}: expected ${version}, got ${actual}.`);
  }
  return version;
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  try {
    console.log(checkVersions());
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

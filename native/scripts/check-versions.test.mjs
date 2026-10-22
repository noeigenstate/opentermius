import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkVersions } from './check-versions.mjs';

test('release version agrees across manifests and npm lockfile', () => {
  assert.match(checkVersions(), /^\d+\.\d+\.\d+$/);
});

test('version check rejects drift, missing package versions and unsupported installer versions', () => {
  const root = mkdtempSync(join(tmpdir(), 'opentermius-versions-'));
  try {
    mkdirSync(join(root, 'desktop'));
    mkdirSync(join(root, 'web'));
    const files = {
      'desktop/tauri.conf.json': '{"version":"0.5.0"}',
      'Cargo.toml': '[package]\nname = "core"\nversion = "0.5.0"\n\n[dependencies]\n',
      'desktop/Cargo.toml': '[package]\nversion = "0.5.0"\n',
      'web/package.json': '{"version":"0.5.0"}',
      'web/package-lock.json': '{"version":"0.5.0","packages":{"":{"version":"0.5.0"}}}',
    };
    const reset = () => {
      for (const [path, text] of Object.entries(files)) writeFileSync(join(root, path), text);
    };
    reset();
    assert.equal(checkVersions(root), '0.5.0');
    for (const [path, text] of Object.entries(files)) {
      writeFileSync(join(root, path), text.replaceAll('0.5.0', '0.4.1'));
      assert.throws(() => checkVersions(root), /expected/);
      reset();
    }
    writeFileSync(join(root, 'Cargo.toml'), '[package]\nname = "core"\n[dependencies]\nversion = "0.5.0"\n');
    assert.throws(() => checkVersions(root), /no package version/);
    reset();
    writeFileSync(join(root, 'desktop/tauri.conf.json'), '{"version":"0.5.0-beta.1"}');
    assert.throws(() => checkVersions(root), /numeric X.Y.Z/);
  } finally {
    rmSync(root, { recursive: true });
  }
});

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const files = ["styles.css", "hosts.css", "workspaces.css"];
const css = Object.fromEntries(await Promise.all(files.map(async (file) => [file, await readFile(new URL(`../src/${file}`, import.meta.url), "utf8")])));

const SPEC = {
  "--icon-default": "#475467",
  "--icon-secondary": "#667085",
  "--icon-disabled": "#98a2b3",
  "--icon-active": "#2563eb",
  "--icon-hover": "#1d4ed8",
  "--icon-success": "#15803d",
  "--icon-warning": "#b45309",
  "--icon-error": "#b42318",
};

test("the :root block defines exactly the eight icon colours of the spec", () => {
  const root = css["styles.css"].match(/^:root \{([^}]*)\}/m)?.[1] ?? "";
  const icons = Object.fromEntries([...root.matchAll(/(--icon-[a-z]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim().toLowerCase()]));
  assert.deepEqual(icons, SPEC);
});

test("rules outside :root use colour tokens only, never raw colours", () => {
  for (const [file, text] of Object.entries(css)) {
    const rules = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^:root \{[^}]*\}/m, "");
    assert.doesNotMatch(rules, /#[0-9a-f]{3,8}\b/i, `${file} holds a hex colour outside :root`);
    assert.doesNotMatch(rules, /\b(?:rgb|hsl)a?\((?!\s*0,\s*0,\s*0\b)/i, `${file} holds a raw colour function outside :root`);
  }
});

test("forced colours remap every icon token to a system colour", () => {
  const block = css["styles.css"].match(/@media \(forced-colors: active\) \{\s*(?:\/\*[\s\S]*?\*\/\s*)?:root \{([^}]*)\}/)?.[1] ?? "";
  for (const name of Object.keys(SPEC)) assert.match(block, new RegExp(`${name}: (?:CanvasText|GrayText|Highlight);`), `${name} keeps a raw colour in forced-colors mode`);
});

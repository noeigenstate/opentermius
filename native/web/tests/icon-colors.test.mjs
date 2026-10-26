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

const declarations = (block, prefix) => Object.fromEntries([...block.matchAll(new RegExp(`(--${prefix}[a-z-]*):\s*([^;]+);`, "g"))].map(([, name, value]) => [name, value.trim().toLowerCase()]));
const lightBlock = css["styles.css"].match(/^:root \{([^}]*)\}/m)?.[1] ?? "";
const darkBlock = css["styles.css"].match(/^:root\[data-theme="dark"\] \{([^}]*)\}/m)?.[1] ?? "";

test("the :root block defines exactly the eight icon colours of the spec", () => {
  assert.deepEqual(declarations(lightBlock, "icon-"), SPEC);
});

test("rules outside the theme blocks use colour tokens only, never raw colours", () => {
  for (const [file, text] of Object.entries(css)) {
    const rules = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^:root(?:\[data-theme="dark"\])? \{[^}]*\}/gm, "");
    assert.doesNotMatch(rules, /#[0-9a-f]{3,8}\b/i, `${file} holds a hex colour outside the theme blocks`);
    assert.doesNotMatch(rules, /\b(?:rgb|hsl)a?\((?!\s*0,\s*0,\s*0\b)/i, `${file} holds a raw colour function outside the theme blocks`);
  }
});

test("the dark theme redefines the icon palette and both themes define the same terminal palette", () => {
  assert.deepEqual(Object.keys(declarations(darkBlock, "icon-")), Object.keys(SPEC));
  const light = declarations(lightBlock, "term-");
  assert.equal(Object.keys(light).length, 22);
  assert.deepEqual(Object.keys(declarations(darkBlock, "term-")), Object.keys(light));
  assert.match(darkBlock, /color-scheme:\s*dark;/);
});

test("forced colours remap every icon token to a system colour, in both themes", () => {
  const block = css["styles.css"].match(/@media \(forced-colors: active\) \{\s*(?:\/\*[\s\S]*?\*\/\s*)?:root, :root\[data-theme="dark"\] \{([^}]*)\}/)?.[1] ?? "";
  for (const name of Object.keys(SPEC)) assert.match(block, new RegExp(`${name}: (?:CanvasText|GrayText|Highlight);`), `${name} keeps a raw colour in forced-colors mode`);
});

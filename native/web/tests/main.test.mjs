import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;

test("document contextmenu blocks browser commands while preserving editing and selected text", async context => {
  const source = await readFile(new URL("../src/main.tsx", import.meta.url), "utf8");
  const listeners = [];
  let selection = "";
  class HTMLElement {
    constructor(tag = "div", parent = null, editable = false) { this.tag = tag; this.parent = parent; this.isContentEditable = editable; }
    closest(selector) {
      assert.equal(selector, "input, textarea");
      return ["input", "textarea"].includes(this.tag) ? this : this.parent?.closest(selector);
    }
  }
  const globals = {
    HTMLElement,
    document: { addEventListener: (...args) => listeners.push(args), getElementById: () => ({}) },
    window: { getSelection: () => selection === null ? null : { toString: () => selection } },
  };
  const originals = Object.keys(globals).map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  for (const [name, value] of Object.entries(globals)) Object.defineProperty(globalThis, name, { configurable: true, value });
  context.after(() => {
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });
  const mock = moduleUrl(`
    export function restoreTheme() {}
    export function createRoot() { return { render() {} }; }
    export function jsx() { return {}; }
    export const jsxs = jsx, StrictMode = "StrictMode";
    export default function App() {}
  `);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/import "[^"]+\.css";/g, "").replace(/from "[^"]+"/g, `from ${JSON.stringify(mock)}`);
  await import(moduleUrl(compiled));
  assert.equal(listeners.length, 1);
  const [type, handler, options] = listeners[0];
  assert.equal(type, "contextmenu");
  assert(!options, "the listener must allow terminal capture handlers to run first");
  for (const [target, text, expected] of [
    [new HTMLElement(), "", true],
    [new HTMLElement("input"), "", false],
    [new HTMLElement("textarea"), "", false],
    [new HTMLElement("span", null, true), "", false],
    [new HTMLElement("span", new HTMLElement("input")), "", false],
    [new HTMLElement(), "selected text", false],
    [new HTMLElement(), " ", false],
    [new HTMLElement(), null, true],
  ]) {
    selection = text;
    let prevented = false;
    handler({ target, preventDefault() { prevented = true; } });
    assert.equal(prevented, expected);
  }
});

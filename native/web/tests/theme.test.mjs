import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/theme.ts", import.meta.url), "utf8");
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;
const settings = (appearance, themeSchedule = false, themeDayStart = "07:00", themeNightStart = "19:00") => ({ appearance, themeSchedule, themeDayStart, themeNightStart });
const at = (hours, minutes = 0) => new Date(2026, 0, 1, hours, minutes);

async function mountTheme(context) {
  const nativeUrl = moduleUrl(`// fixture ${++fixture}
    export const backgrounds = [];
    export function getCurrentWindow() { return { async setBackgroundColor(value) { backgrounds.push(value); } }; }
  `);
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
    .replaceAll(JSON.stringify("@tauri-apps/api/window"), JSON.stringify(nativeUrl));
  const theme = await import(moduleUrl(compiled));
  const native = await import(nativeUrl);
  const root = { dataset: {} };
  const document = new EventTarget();
  document.documentElement = root;
  const window = new EventTarget();
  const media = new EventTarget();
  media.matches = false;
  const listeners = new Set();
  const add = media.addEventListener.bind(media), remove = media.removeEventListener.bind(media);
  media.addEventListener = (name, callback) => { listeners.add(callback); add(name, callback); };
  media.removeEventListener = (name, callback) => { listeners.delete(callback); remove(name, callback); };
  window.matchMedia = query => { assert.equal(query, "(prefers-color-scheme: dark)"); return media; };
  const timers = new Map(), observers = new Set(), stored = new Map();
  window.setInterval = (callback, delay) => { const id = timers.size + 1; timers.set(id, { callback, delay }); return id; };
  window.clearInterval = id => timers.delete(id);
  const globals = {
    window, document,
    localStorage: { getItem: key => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) },
    getComputedStyle: () => ({ getPropertyValue: name => { assert.equal(name, "--surface"); return ` surface-${root.dataset.theme} `; } }),
    MutationObserver: class {
      constructor(callback) { this.callback = callback; observers.add(this); }
      observe(element, options) { assert.equal(element, root); assert.deepEqual(options, { attributes: true, attributeFilter: ["data-window-background"] }); }
      disconnect() { observers.delete(this); }
    },
  };
  const originals = Object.keys(globals).map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  for (const [name, value] of Object.entries(globals)) Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  const cleanups = [];
  context.after(() => {
    for (const cleanup of cleanups) cleanup();
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name];
    }
  });
  return {
    ...theme, native, root, media, window, document, timers, observers, stored, listeners,
    follow(value) { const cleanup = theme.followTheme(value); cleanups.push(cleanup); return cleanup; },
    system(dark) { media.matches = dark; media.dispatchEvent(new Event("change")); },
    background(value) { root.dataset.windowBackground = value; for (const observer of observers) observer.callback(); },
    tick() { for (const { callback } of timers.values()) callback(); },
  };
}

test("explicit, system and scheduled themes resolve with exact boundaries and midnight wrap", async context => {
  const { resolveTheme } = await mountTheme(context);
  for (const system of [false, true]) {
    assert.equal(resolveTheme(settings("light"), system), "light");
    assert.equal(resolveTheme(settings("dark"), system), "dark");
    assert.equal(resolveTheme(settings("system"), system), system ? "dark" : "light");
  }
  for (const [hours, minutes, expected] of [[6, 59, "dark"], [7, 0, "light"], [18, 59, "light"], [19, 0, "dark"], [0, 0, "dark"]]) {
    for (const appearance of ["light", "dark", "system"]) {
      for (const system of [false, true]) assert.equal(resolveTheme(settings(appearance, true), system, at(hours, minutes)), expected);
    }
  }
  for (const [hours, expected] of [[0, "light"], [5, "light"], [6, "dark"], [19, "dark"], [20, "light"]]) {
    assert.equal(resolveTheme(settings("system", true, "20:00", "06:00"), false, at(hours)), expected);
  }
  for (const [day, night] of [["", "19:00"], ["24:00", "19:00"], ["07:00", "12:60"], ["7:00", "19:00"], ["07:00", "07:00"]]) {
    assert.equal(resolveTheme(settings("dark", true, day, night), false, at(7)), "light");
    assert.equal(resolveTheme(settings("light", true, day, night), true, at(19)), "dark");
  }
});

test("system changes apply and remember only resolved themes, and listeners are released", async context => {
  const page = await mountTheme(context);
  const cleanup = page.follow(settings("system"));
  assert.equal(page.root.dataset.theme, "light");
  assert.equal(page.listeners.size, 1);
  page.system(true);
  assert.equal(page.root.dataset.theme, "dark");
  assert.deepEqual([...page.stored.values()], ["dark"]);
  page.system(false);
  assert.equal(page.root.dataset.theme, "light");
  cleanup();
  assert.equal(page.listeners.size, 0);
  assert.equal(page.observers.size, 0);
  page.system(true);
  assert.equal(page.root.dataset.theme, "light");
  const explicit = page.follow(settings("dark"));
  assert.equal(page.listeners.size, 0);
  assert.equal(page.root.dataset.theme, "dark");
  assert.equal(page.timers.size, 0);
  explicit();
});

test("remembered theme is applied before React renders and tolerates unavailable storage", async context => {
  const page = await mountTheme(context);
  const key = "opentermius-theme";
  for (const [remembered, expected] of [["dark", "dark"], ["light", "light"], ["system", "light"], [null, "light"]]) {
    page.stored.set(key, remembered);
    page.restoreTheme();
    assert.equal(page.root.dataset.theme, expected);
  }
  const main = await readFile(new URL("../src/main.tsx", import.meta.url), "utf8");
  const mock = moduleUrl(`
    export function restoreTheme() { document.documentElement.dataset.theme = "dark"; }
    export function createRoot() { if (document.documentElement.dataset.theme !== "dark") throw Error("rendered before restoring theme"); return { render() {} }; }
    export function jsx() { return {}; }
    export const jsxs = jsx, StrictMode = "StrictMode";
    export default function App() {}
  `);
  page.root.dataset.theme = "light";
  page.document.getElementById = () => ({});
  const compiled = ts.transpileModule(main, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX } }).outputText
    .replace(/import "[^\"]+\.css";/g, "").replace(/from "[^\"]+"/g, `from ${JSON.stringify(mock)}`);
  await import(moduleUrl(compiled));
  assert.equal(page.root.dataset.theme, "dark");
  Object.defineProperty(globalThis, "localStorage", { configurable: true, get() { throw Error("storage unavailable"); } });
  assert.doesNotThrow(() => page.restoreTheme());
  assert.equal(page.root.dataset.theme, "light");
  assert.doesNotThrow(() => page.follow(settings("dark")));
  assert.equal(page.root.dataset.theme, "dark");
});

test("schedule refreshes each minute, on focus and visibility, and cleans up", async context => {
  const page = await mountTheme(context);
  const RealDate = Date;
  let now = at(6, 59);
  context.after(() => { globalThis.Date = RealDate; });
  globalThis.Date = class extends RealDate { constructor() { super(now); } };
  const cleanup = page.follow(settings("system", true));
  assert.equal(page.listeners.size, 0);
  assert.equal(page.root.dataset.theme, "dark");
  assert.deepEqual([...page.timers.values()].map(timer => timer.delay), [60_000]);
  now = new RealDate(2026, 0, 1, 7);
  page.tick();
  assert.equal(page.root.dataset.theme, "light");
  now = new RealDate(2026, 0, 1, 19);
  page.window.dispatchEvent(new Event("focus"));
  assert.equal(page.root.dataset.theme, "dark");
  now = new RealDate(2026, 0, 2, 7);
  page.document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(page.root.dataset.theme, "light");
  cleanup();
  assert.equal(page.timers.size, 0);
  now = new RealDate(2026, 0, 2, 19);
  page.window.dispatchEvent(new Event("focus"));
  page.document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(page.root.dataset.theme, "light");
});

test("native background uses the CSS surface only after Rust reports an opaque window", async context => {
  const page = await mountTheme(context);
  const cleanup = page.follow(settings("system"));
  page.background("transparent");
  page.root.dataset.windowShape = "square";
  page.system(true);
  assert.deepEqual(page.native.backgrounds, []);
  page.background("opaque");
  assert.deepEqual(page.native.backgrounds, ["surface-dark"]);
  page.system(false);
  assert.deepEqual(page.native.backgrounds, ["surface-dark", "surface-light"]);
  cleanup();
  page.background("opaque");
  assert.equal(page.native.backgrounds.length, 2);
});

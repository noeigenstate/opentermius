import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Logs.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;

// Exercise the real page effect, replacing its hooks and native boundary only.
async function mountPage(context, props = {}) {
  const timers = new Map();
  const listeners = new Set();
  let nextTimer = 0;
  const document = new EventTarget();
  document.visibilityState = "visible";
  const addListener = document.addEventListener.bind(document);
  const removeListener = document.removeEventListener.bind(document);
  document.addEventListener = (type, listener) => {
    if (type === "visibilitychange") listeners.add(listener);
    addListener(type, listener);
  };
  document.removeEventListener = (type, listener) => {
    if (type === "visibilitychange") listeners.delete(listener);
    removeListener(type, listener);
  };
  const window = {
    setInterval(callback, delay) { const id = ++nextTimer; timers.set(id, { callback, delay }); return id; },
    clearInterval(id) { timers.delete(id); },
  };
  const originals = ["window", "document"].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  let cleanups = [];
  const unmount = () => { for (const cleanup of cleanups.splice(0)) cleanup(); };
  context.after(() => {
    unmount();
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });
  Object.defineProperty(globalThis, "window", { configurable: true, writable: true, value: window });
  Object.defineProperty(globalThis, "document", { configurable: true, writable: true, value: document });

  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    export const effects = [];
    export const writes = [];
    export const requests = [];
    export function useEffect(effect) { effects.push(effect); }
    export function useState(initial) {
      return [typeof initial === "function" ? initial() : initial, (value) => writes.push(value)];
    }
    export function listLogs() {
      return new Promise((resolve, reject) => requests.push({ resolve, reject }));
    }
    export async function clearLogs() {}
    export async function confirm() { return false; }
    export function t(text) { return text; }
    export function Icon() { return null; }
    export function jsx(type, props) { return { type, props }; }
    export const jsxs = jsx;
  `);
  let compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  for (const specifier of ["react", "react/jsx-runtime", "@tauri-apps/plugin-dialog", "../api/logs", "../i18n", "../icons"]) {
    compiled = compiled.replaceAll(JSON.stringify(specifier), JSON.stringify(mockUrl));
  }
  const mock = await import(mockUrl);
  const { LogsPage } = await import(moduleUrl(compiled));
  LogsPage(props);
  cleanups = mock.effects.map((effect) => effect()).filter((cleanup) => typeof cleanup === "function");
  return {
    mock, timers, listeners, unmount,
    tick() { for (const { callback } of [...timers.values()]) callback(); },
    visibility(value) { document.visibilityState = value; document.dispatchEvent(new Event("visibilitychange")); },
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

test("visible history refreshes immediately, pauses while hidden and resumes with one timer", async (context) => {
  const page = await mountPage(context);
  assert.equal(page.mock.requests.length, 1, "mount must request history immediately");
  assert.deepEqual([...page.timers.values()].map(({ delay }) => delay), [5000]);
  assert.equal(page.listeners.size, 1);
  page.mock.requests[0].resolve([]);
  await settle();

  page.visibility("hidden");
  assert.equal(page.timers.size, 0, "hidden documents must release their polling timer");
  page.tick();
  assert.equal(page.mock.requests.length, 1);

  page.visibility("visible");
  assert.equal(page.mock.requests.length, 2, "resuming must refresh without waiting five seconds");
  assert.deepEqual([...page.timers.values()].map(({ delay }) => delay), [5000]);
  page.mock.requests[1].resolve([]);
  await settle();
  page.tick();
  assert.equal(page.mock.requests.length, 3, "one interval must produce one refresh");

  page.unmount();
  assert.equal(page.timers.size, 0);
  assert.equal(page.listeners.size, 0, "unmount must release the visibility listener");
  page.visibility("visible");
  page.tick();
  assert.equal(page.mock.requests.length, 3, "unmounted pages must remain idle");
  page.mock.requests[2].resolve([]);
  await settle();
});

test("a page hidden by navigation performs no IPC and registers no polling resources", async (context) => {
  const page = await mountPage(context, { visible: false });
  assert.equal(page.mock.requests.length, 0);
  assert.equal(page.timers.size, 0);
  assert.equal(page.listeners.size, 0);
  page.visibility("hidden");
  page.visibility("visible");
  page.tick();
  assert.equal(page.mock.requests.length, 0, "document visibility cannot reactivate an inactive page");
  assert.equal(page.mock.writes.length, 0);
});

test("overlapping polling avoids duplicate IPC and unmount ignores pending results", async (context) => {
  for (const outcome of ["resolve", "reject"]) {
    await context.test(`late ${outcome}`, async (context) => {
      const page = await mountPage(context);
      page.tick();
      page.tick();
      assert.equal(page.mock.requests.length, 1, "a pending request must block overlapping interval requests");

      page.mock.requests[0].resolve([]);
      await settle();
      page.tick();
      assert.equal(page.mock.requests.length, 2, "polling must continue when the prior request completes");
      page.unmount();
      const writeCount = page.mock.writes.length;
      page.mock.requests[1][outcome](outcome === "resolve" ? [{ id: "fixture-session" }] : new Error("Fixture IPC failure"));
      await settle();
      assert.equal(page.mock.writes.length, writeCount, "pending success, error and loading updates must not reach an unmounted page");
      assert.equal(page.timers.size, 0);
      assert.equal(page.listeners.size, 0);
    });
  }
});

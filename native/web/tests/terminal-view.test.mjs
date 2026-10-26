import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/components/TerminalView.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;

// Run the real component effects across renders with stable refs and mocked browser/xterm boundaries.
async function mountTerminal(context, initial = {}) {
  const frames = new Map();
  let nextFrame = 0;
  const observers = new Set();
  const themeObservers = new Set();
  const root = { dataset: { theme: "light" } };
  const globals = {
    document: { documentElement: root },
    getComputedStyle(element) {
      assert.equal(element, root);
      return { getPropertyValue(name) { return ` ${root.dataset.theme}:${name} `; } };
    },
    MutationObserver: class {
      constructor(callback) { this.callback = callback; themeObservers.add(this); }
      observe(element, options) { assert.equal(element, root); assert.deepEqual(options, { attributes: true, attributeFilter: ["data-theme"] }); }
      disconnect() { themeObservers.delete(this); }
    },
    requestAnimationFrame(callback) { const id = ++nextFrame; frames.set(id, callback); return id; },
    cancelAnimationFrame(id) { frames.delete(id); },
    ResizeObserver: class {
      constructor(callback) { this.callback = callback; observers.add(this); }
      observe() {}
      disconnect() { observers.delete(this); }
    },
  };
  const originals = Object.keys(globals).map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  }
  let mock;
  context.after(() => {
    mock?.unmount();
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });

  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    let cursor = 0;
    const slots = [];
    const effects = [];
    const subscribers = new Set();
    export const instances = [];
    export const sizes = [];
    export const calls = [];
    export const element = { clientWidth: 900, clientHeight: 500, addEventListener() {}, removeEventListener() {} };
    export function begin() { cursor = 0; }
    export function useRef(initial) {
      const index = cursor++;
      return slots[index] ??= { current: initial };
    }
    export function useState(initial) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], value => { slots[index] = value; }];
    }
    export function useEffect(effect, deps) {
      const index = cursor++;
      const previous = slots[index];
      if (!previous || deps.some((value, i) => value !== previous.deps[i])) {
        effects.push(() => { previous?.cleanup?.(); slots[index] = { deps, cleanup: effect() }; });
      }
    }
    export function commit() { for (const effect of effects.splice(0)) effect(); }
    export function unmount() { for (const slot of slots) slot?.cleanup?.(); }
    export function emit(data) { for (const listener of subscribers) listener({ type: "output", data }); }
    export function subscribeSession(id, listener) { subscribers.add(listener); return () => subscribers.delete(listener); }
    export async function sessionInput() {}
    export async function sessionResize(id, cols, rows) { sizes.push([cols, rows]); }
    export class Terminal {
      cols = 100;
      rows = 30;
      buffer = { active: { viewportY: 10, baseY: 10 } };
      writes = [];
      refreshes = [];
      scrolls = 0;
      opens = 0;
      disposed = false;
      measured = false;
      constructor(options) { this.options = options; instances.push(this); }
      loadAddon(addon) { addon.activate?.(this); }
      open() { this.opens++; this.measured = element.clientWidth > 0 && element.clientHeight > 0; }
      resize(cols, rows) {
        calls.push("measure");
        // CoreBrowserTerminal.resize remeasures invalid character metrics even for unchanged rows/cols.
        if (element.clientWidth && element.clientHeight) this.measured = true;
        this.cols = cols; this.rows = rows;
      }
      refresh(start, end) { calls.push("refresh"); this.refreshes.push([start, end]); }
      scrollToBottom() { calls.push("scroll"); this.scrolls++; this.buffer.active.viewportY = this.buffer.active.baseY; }
      focus() { calls.push("focus"); }
      attachCustomKeyEventHandler() {}
      onData() { return { dispose() {} }; }
      onBinary() { return { dispose() {} }; }
      write(data) { this.writes.push(data); }
      dispose() { this.disposed = true; }
    }
    export class FitAddon {
      dims = { cols: 100, rows: 30 };
      fits = 0;
      activate(terminal) { this.terminal = terminal; terminal.fit = this; }
      proposeDimensions() { return this.terminal.measured ? this.dims : undefined; }
      fit() {
        calls.push("fit"); this.fits++;
        const dims = this.proposeDimensions();
        // FitAddon.fit does nothing if character metrics are zero or the geometry is unchanged.
        if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return;
        if (this.terminal.cols !== dims.cols || this.terminal.rows !== dims.rows) {
          this.terminal.resize(dims.cols, dims.rows);
        }
      }
    }
    export class WebLinksAddon {}
    export function jsx(type, props) { if (props.ref) props.ref.current = element; return { type, props }; }
    export const jsxs = jsx;
    export function t(text) { return text; }
    export const Icon = "Icon";
    export const Dialog = "Dialog";
  `);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/import "[^\"]+\.css";/g, "").replace(/from "[^\"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  mock = await import(mockUrl);
  const { TerminalView } = await import(moduleUrl(compiled));
  let props = { sessionId: "fixture-session", visible: true, ended: false, local: false, blocked: false, onReconnect() {}, ...initial };
  function render(next = {}) {
    props = { ...props, ...next };
    mock.element.clientWidth = props.visible ? 900 : 0;
    mock.element.clientHeight = props.visible ? 500 : 0;
    mock.begin();
    TerminalView(props);
    mock.commit();
  }
  render();
  return {
    mock, frames, render, themeObservers,
    theme(value) { root.dataset.theme = value; for (const observer of themeObservers) observer.callback(); },
    terminal: mock.instances[0],
    resize() { for (const observer of observers) observer.callback(); },
    flush() { const pending = [...frames.values()]; frames.clear(); for (const callback of pending) callback(); },
  };
}

test("terminal palette comes from CSS and changes in place with the resolved app theme", async (context) => {
  const page = await mountTerminal(context);
  const colors = {
    background: "background", foreground: "foreground", cursor: "cursor", cursorAccent: "cursor-accent",
    selectionBackground: "selection-background", selectionForeground: "selection-foreground",
    black: "black", red: "red", green: "green", yellow: "yellow", blue: "blue", magenta: "magenta", cyan: "cyan", white: "white",
    brightBlack: "bright-black", brightRed: "bright-red", brightGreen: "bright-green", brightYellow: "bright-yellow",
    brightBlue: "bright-blue", brightMagenta: "bright-magenta", brightCyan: "bright-cyan", brightWhite: "bright-white",
  };
  const expected = theme => Object.fromEntries(Object.entries(colors).map(([key, token]) => [key, `${theme}:--term-${token}`]));
  assert.deepEqual(page.terminal.options.theme, expected("light"));
  page.render({ visible: false });
  page.theme("dark");
  assert.deepEqual(page.terminal.options.theme, expected("dark"));
  assert.equal(page.mock.instances.length, 1);
  page.theme("light");
  assert.deepEqual(page.terminal.options.theme, expected("light"));
  page.mock.unmount();
  assert.equal(page.themeObservers.size, 0);
});

test("returning to a terminal repaints every row at unchanged geometry without new output or remounting", async (context) => {
  const page = await mountTerminal(context);
  page.flush();
  const terminal = page.terminal;
  const before = terminal.refreshes.length;
  page.render({ visible: false });
  const output = new TextEncoder().encode("fixture output\r\nfixture> ");
  page.mock.emit(output);
  page.resize();
  page.flush();
  assert.deepEqual(terminal.writes, [output], "hidden terminals must keep receiving output");
  assert.equal(terminal.refreshes.length, before);
  page.render({ visible: true });
  page.flush();
  assert.equal(terminal.refreshes.length, before + 1, "same-size fit must still repaint on reveal");
  assert.deepEqual(terminal.refreshes.at(-1), [0, terminal.rows - 1]);
  assert.equal(terminal.scrolls, 2, "a terminal following output stays at the bottom on reveal");
  assert.equal(page.mock.instances.length, 1);
  assert.equal(terminal.opens, 1);
  assert.equal(terminal.disposed, false);
});

test("visible fitting preserves scrollback position and refreshes ended or blocked terminals", async (context) => {
  const page = await mountTerminal(context);
  page.flush();
  page.terminal.buffer.active.viewportY = 4;
  page.render({ visible: false });
  page.render({ visible: true, ended: true, blocked: true });
  page.mock.calls.length = 0;
  page.flush();
  assert.equal(page.terminal.buffer.active.viewportY, 4);
  assert.equal(page.terminal.scrolls, 1, "reading scrollback must not jump to the live prompt");
  assert.equal(page.terminal.refreshes.length, 2);
  assert.equal(page.mock.sizes.length, 1, "ended sessions must not resize their PTY");
  assert.equal(page.mock.calls.includes("focus"), false, "blocked terminals must not steal focus");
});

test("a terminal opened while hidden recovers character measurement before its first visible fit", async (context) => {
  const page = await mountTerminal(context, { visible: false });
  page.mock.emit(new TextEncoder().encode("fixture> "));
  page.flush();
  assert.equal(page.terminal.measured, false);
  assert.equal(page.mock.sizes.length, 0);
  page.render({ visible: true });
  page.terminal.fit.dims = { cols: 110, rows: 32 };
  page.flush();
  assert.equal(page.terminal.measured, true);
  assert.deepEqual(page.mock.sizes, [[110, 32]], "the first PTY resize must use measured geometry");
  assert.deepEqual(page.terminal.refreshes, [[0, 31]]);
  assert.equal(page.terminal.writes.length, 1);
  assert(page.mock.calls.indexOf("measure") < page.mock.calls.indexOf("fit"));
  assert(page.mock.calls.indexOf("scroll") < page.mock.calls.indexOf("refresh"));
});

test("hidden, zero-size and implausible fits never resize the terminal buffer or PTY", async (context) => {
  const page = await mountTerminal(context);
  // Hide with a visible fit still queued; its callback must recheck current visibility.
  page.render({ visible: false });
  page.flush();
  page.resize();
  page.flush();
  assert.equal(page.mock.sizes.length, 0);
  assert.equal(page.terminal.fit.fits, 0);
  assert.deepEqual(page.mock.calls, [], "hidden terminals must not measure, fit, repaint or focus");
  page.render({ visible: true });
  page.mock.element.clientHeight = 0;
  page.flush();
  assert.equal(page.mock.sizes.length, 0);
  assert.deepEqual(page.mock.calls, [], "zero-size containers must not resize even to the current geometry");
  page.mock.element.clientHeight = 500;
  for (const dims of [undefined, { cols: 2, rows: 1 }, { cols: 0, rows: 0 }, { cols: NaN, rows: 30 }, { cols: 100, rows: Infinity }]) {
    page.terminal.fit.dims = dims;
    page.resize();
    page.flush();
    assert.equal(page.mock.sizes.length, 0);
    assert.equal(page.terminal.fit.fits, 0, "invalid proposals must not reflow the buffer");
  }
  page.terminal.fit.dims = { cols: 100, rows: 30 };
  page.resize();
  page.resize();
  assert.equal(page.frames.size, 1, "resize callbacks coalesce into one frame");
  page.flush();
  assert.deepEqual(page.mock.sizes, [[100, 30]]);
  page.resize();
  page.mock.unmount();
  assert.equal(page.frames.size, 0);
  page.flush();
  assert.equal(page.mock.sizes.length, 1);
});

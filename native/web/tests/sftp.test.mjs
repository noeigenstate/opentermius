import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Sftp.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const flush = () => new Promise((resolve) => setImmediate(resolve));
let fixture = 0;

// Render the real workspace and panes with stateful hooks and native IPC isolated.
async function createPage(context) {
  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    const scopes = new Map();
    let slots, cursor;
    export const effects = [], calls = [], clipboard = [];
    export let handlers, clipboardFailure = false;
    export function failClipboard() { clipboardFailure = true; }
    export function begin(scope) {
      if (!scopes.has(scope)) scopes.set(scope, []);
      slots = scopes.get(scope); cursor = 0;
    }
    export function useState(initial) {
      const state = slots, index = cursor++;
      if (!(index in state)) state[index] = typeof initial === "function" ? initial() : initial;
      return [state[index], value => { state[index] = typeof value === "function" ? value(state[index]) : value; }];
    }
    export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
    export function useMemo(compute) { return compute(); }
    export function useEffect(effect, deps) {
      const index = cursor++;
      if (!slots[index] || deps.some((value, i) => value !== slots[index][i])) effects.push(effect);
      slots[index] = deps;
    }
    export function useSyncExternalStore(subscribe, snapshot) { return snapshot(); }
    export function jsx(type, props) { return { type, props }; }
    export const jsxs = jsx, Fragment = "Fragment", Menu = "Menu";
    export function t(text) { return text; }
    export function Icon() {}
    export const entries = [
      { name: "other.txt", isDir: false, size: 10, modified: 0 },
      { name: "docs", isDir: true, size: 0, modified: 0 },
      { name: "report.txt", isDir: false, size: 20, modified: 0 },
    ];
    export const localPath = "E:\\\\sftp-fixture";
    export async function localHome() { return localPath; }
    export async function localList(path) {
      calls.push(["localList", path]);
      return { path, parent: "E:\\\\", entries: entries.map(entry => ({ ...entry, path: path + "\\\\" + entry.name })) };
    }
    export async function listenSftp(id, next) { handlers = next; return () => {}; }
    export function files(path = "/fixture") {
      handlers.files({ path, entries: entries.map(entry => ({ ...entry, path: path.replace(/\\/+$/, "") + "/" + entry.name })) });
    }
    export async function remoteList(id, path) { calls.push(["remoteList", path]); files(path === "." ? "/fixture" : path); }
    export async function upload(...args) { calls.push(["upload", ...args]); }
    export async function download(...args) { calls.push(["download", ...args]); }
    export async function remoteDelete(...args) { calls.push(["remoteDelete", ...args]); }
    export async function localDelete(...args) { calls.push(["localDelete", ...args]); return { files: 1, dirs: 0, bytes: 20 }; }
  `);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  const { SftpView } = await import(moduleUrl(compiled));
  const mock = await import(mockUrl);
  const original = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { async writeText(text) {
    if (mock.clipboardFailure) throw Error("clipboard denied");
    mock.clipboard.push(text);
  } } } });
  context.after(() => {
    if (original) Object.defineProperty(globalThis, "navigator", original);
    else delete globalThis.navigator;
  });
  const render = () => {
    mock.begin("workspace");
    const workspace = SftpView({ sessionId: "fixture-session" });
    return workspace.type(workspace.props);
  };
  const pane = (side) => {
    const node = nodes(render()).find(node => node.props?.side === side);
    const renderPane = () => { mock.begin(side); return node.type(node.props); };
    return { node, render: renderPane, tree: renderPane() };
  };
  render();
  for (const effect of mock.effects.splice(0)) effect();
  await flush();
  const menu = (side, name) => {
    const current = pane(side);
    const target = name ? nodes(current.tree).find(node => node.type === "tr" && node.props.onContextMenu && nodes(node).some(child => child.props?.title === name))
      : nodes(current.tree).find(node => node.props?.className === "sftp-table-wrap");
    let prevented = false, stopped = false;
    target.props.onContextMenu({ clientX: 120, clientY: 240, preventDefault() { prevented = true; }, stopPropagation() { stopped = true; } });
    assert(prevented);
    assert.equal(stopped, Boolean(name), "row events must not replace the menu with the empty-space menu");
    // Keep the workspace callback from before onSelect's state update.
    const opened = nodes(current.render()).find(node => node.type === mock.Menu);
    assert.equal(opened.props.x, 120);
    assert.equal(opened.props.y, 240);
    return opened.props;
  };
  return { mock, render, pane, menu };
}

function nodes(node) {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!node || typeof node !== "object") return [];
  return [node, ...nodes(node.props?.children)];
}

test("folder and file menus on each side have the required order, icons and destructive item", async context => {
  const page = await createPage(context);
  for (const side of ["local", "remote"]) {
    for (const [name, folder] of [["docs", true], ["report.txt", false]]) {
      const { items, onClose } = page.menu(side, name);
      assert.deepEqual(items.map(item => item.label), [...(folder ? ["Open"] : []), side === "local" ? "Upload" : "Download", "Copy name", "Copy path", "Rename", "Delete"]);
      assert.deepEqual(items.map(item => item.icon), [...(folder ? ["folder"] : []), side === "local" ? "arrowUp" : "download", "copy", "copy", "edit", "trash"]);
      assert.equal(items.at(-1).danger, true);
      assert(items.every(item => !item.disabled));
      assert.equal(page.pane(side).node.props.selected, name);
      onClose();
      assert(!nodes(page.pane(side).tree).some(node => node.type === page.mock.Menu));
    }
  }
});

test("row actions use the right-clicked entry while the workspace still has another selection", async context => {
  const page = await createPage(context);
  for (const side of ["local", "remote"]) {
    for (const label of [side === "local" ? "Upload" : "Download", "Rename", "Delete"]) {
      page.pane(side).node.props.onSelect("other.txt");
      const { items } = page.menu(side, "report.txt");
      items.find(item => item.label === label).onSelect();
      await flush();
      if (label === "Rename") {
        const input = nodes(page.render()).find(node => node.type === "input" && node.props.autoFocus);
        assert.equal(input.props.value, "report.txt");
        nodes(page.render()).find(node => node.type === "button" && node.props.children === "Cancel").props.onClick();
      } else if (label === "Delete") {
        const call = page.mock.calls.findLast(call => call[0] === (side === "local" ? "localDelete" : "remoteDelete"));
        assert.equal(call.at(-1), "report.txt");
        if (side === "local") nodes(page.render()).find(node => node.type === "button" && node.props.children === "Cancel").props.onClick();
      } else {
        const call = page.mock.calls.findLast(call => call[0] === (side === "local" ? "upload" : "download"));
        assert.equal(call[side === "local" ? 4 : 3], "report.txt");
      }
    }
    const { items } = page.menu(side, "docs");
    items[0].onSelect();
    await flush();
    assert.deepEqual(page.mock.calls.at(-1), [side === "local" ? "localList" : "remoteList", side === "local" ? `${page.mock.localPath}\\docs` : "/fixture/docs"]);
  }
});

test("clipboard copies names, local full paths and remote root, trailing-slash and relative paths", async context => {
  const page = await createPage(context);
  for (const side of ["local", "remote"]) {
    const { items } = page.menu(side, "report.txt");
    items.find(item => item.label === "Copy name").onSelect();
    items.find(item => item.label === "Copy path").onSelect();
    await flush();
    assert.deepEqual(page.mock.clipboard.slice(-2), ["report.txt", side === "local" ? `${page.mock.localPath}\\report.txt` : "/fixture/report.txt"]);
  }
  for (const [path, expected] of [["/", "/report.txt"], ["/fixture///", "/fixture/report.txt"], [".", "./report.txt"], ["relative/docs", "relative/docs/report.txt"], ["../docs/", "../docs/report.txt"], ["", "report.txt"]]) {
    page.mock.files(path);
    page.menu("remote", "report.txt").items.find(item => item.label === "Copy path").onSelect();
    await flush();
    assert.equal(page.mock.clipboard.at(-1), expected);
  }
  page.mock.failClipboard();
  page.menu("local", "report.txt").items.find(item => item.label === "Copy name").onSelect();
  await flush();
  assert(nodes(page.render()).some(node => node.props?.role === "alert" && nodes(node).some(child => child.props?.children === "Cannot access clipboard")));
});

test("empty-space menus preserve selection and use existing new-folder and refresh actions", async context => {
  const page = await createPage(context);
  for (const side of ["local", "remote"]) {
    page.pane(side).node.props.onSelect("other.txt");
    const { items } = page.menu(side);
    assert.deepEqual(items.map(item => [item.label, item.icon]), [["New folder", "plus"], ["Refresh", "refresh"]]);
    assert.equal(page.pane(side).node.props.selected, "other.txt");
    items[0].onSelect();
    assert(nodes(page.render()).some(node => node.type === "h2" && node.props.children === "New folder"));
    nodes(page.render()).find(node => node.type === "button" && node.props.children === "Cancel").props.onClick();
    items[1].onSelect();
    await flush();
    assert.deepEqual(page.mock.calls.at(-1), [side === "local" ? "localList" : "remoteList", side === "local" ? page.mock.localPath : "/fixture"]);
  }
});

test("offline and loading menu disabled states match existing controls", async context => {
  const page = await createPage(context);
  // Keep an open remote row menu across a disconnect.
  page.menu("remote", "docs");
  page.mock.handlers.status({ connected: false });
  const remote = page.pane("remote");
  const items = nodes(remote.tree).find(node => node.type === page.mock.Menu).props.items;
  assert.deepEqual(items.map(item => Boolean(item.disabled)), [true, true, false, false, true, true]);
  assert.equal(Boolean(page.menu("local", "report.txt").items[0].disabled), true);
  assert(page.menu("remote").items.every(item => item.disabled));
  page.mock.files();
  page.pane("remote").node.props.onNavigate("/fixture");
  // remoteList resolves immediately in the fixture; local loading is still observable synchronously.
  page.pane("local").node.props.onNavigate(page.mock.localPath);
  assert.equal(Boolean(page.menu("local").items[1].disabled), true);
  await flush();
});

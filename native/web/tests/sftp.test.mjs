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
    let ids = 0;
    export function useId() { return useState(() => "id" + ++ids)[0]; }
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
      { name: ".github", isDir: true, size: 0, modified: 0 },
      { name: ".gitignore", isDir: false, size: 5, modified: 0 },
      { name: ".gitattributes", isDir: false, size: 5, modified: 0 },
      { name: ".gitkeep", isDir: false, size: 0, modified: 0 },
      { name: "my.gitignore", isDir: false, size: 5, modified: 0 },
      { name: "README.md", isDir: false, size: 5, modified: 0 },
      { name: "LICENSE", isDir: false, size: 5, modified: 0 },
      { name: "readme-notes.bin", isDir: false, size: 5, modified: 0 },
      { name: "archive.tar.gz", isDir: false, size: 5, modified: 0 },
    ];
    export const localPath = "E:\\\\sftp-fixture";
    export async function localHome() { return localPath; }
    let places = null, placesFailure = false;
    export function setPlaces(next) { places = next; }
    export function failPlaces() { placesFailure = true; }
    export async function localPlaces() {
      calls.push(["localPlaces"]);
      if (placesFailure) throw Error("places denied");
      return places ?? [
        { kind: "home", path: localPath, name: null }, { kind: "desktop", path: localPath + "\\\\Desktop", name: null },
        { kind: "downloads", path: localPath + "\\\\Downloads", name: null }, { kind: "documents", path: localPath + "\\\\Documents", name: null },
        { kind: "drive", path: "C:\\\\", name: null }, { kind: "drive", path: "D:\\\\", name: "Data" },
      ];
    }
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

test("folders, git files, documents and other files get their glyph and tone on both panes", async context => {
  const page = await createPage(context);
  nodes(page.render()).find(node => node.props?.role === "switch").props.onChange({ target: { checked: true } });
  const expected = {
    docs: ["folder", undefined], ".github": ["folder", undefined],
    ".gitignore": ["gitBranch", "git"], ".gitattributes": ["gitBranch", "git"], ".gitkeep": ["gitBranch", "git"],
    "README.md": ["fileText", "doc"], "report.txt": ["fileText", "doc"], LICENSE: ["fileText", "doc"],
    "my.gitignore": ["file", undefined], "readme-notes.bin": ["file", undefined], "archive.tar.gz": ["file", undefined],
  };
  for (const side of ["local", "remote"]) {
    const tree = page.pane(side).tree;
    for (const [name, glyph] of Object.entries(expected)) {
      const button = nodes(tree).find(node => node.type === "button" && node.props.title === name);
      assert.deepEqual([nodes(button).find(node => node.type === page.mock.Icon).props.name, button.props["data-tone"]], glyph, `${side} ${name}`);
    }
  }
});

// Render the local pane's real PlacesField with its own hook scope; handlers are re-read after every render.
function placesField(page) {
  const node = nodes(page.pane("local").tree).find(item => item.type?.name === "PlacesField");
  const render = () => { page.mock.begin("places"); return node.type(node.props); };
  const input = () => nodes(render()).find(item => item.type === "input");
  const listbox = () => nodes(render()).find(item => item.props?.role === "listbox");
  const options = () => nodes(listbox() ?? []).filter(item => item.props?.role === "option");
  const label = option => nodes(option).find(item => item.type === "span").props.children;
  const rows = () => nodes(listbox() ?? []).filter(item => item.type === "hr" || item.props?.role === "option").map(item => item.type === "hr" ? "---" : label(item));
  const press = (key, extra = {}) => { let prevented = false; input().props.onKeyDown({ key, ...extra, preventDefault() { prevented = true; } }); return prevented; };
  const selected = () => options().findIndex(option => option.props["aria-selected"]);
  return { node, render, input, listbox, options, rows, press, selected };
}

test("the local path field is a combobox listing known folders, a separator, then the drives", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  assert.equal(page.pane("remote").node.props.side, "remote");
  assert(!nodes(page.pane("remote").tree).some(node => node.type?.name === "PlacesField"), "the remote pane keeps its plain input");
  assert.equal(field.input().props.role, "combobox");
  assert.equal(field.input().props["aria-expanded"], false);
  assert.equal(field.listbox(), undefined);
  assert.equal(field.press("ArrowDown"), true);
  await flush();
  assert.equal(page.mock.calls.filter(call => call[0] === "localPlaces").length, 1);
  assert.equal(field.input().props["aria-expanded"], true);
  assert.deepEqual(field.rows(), ["Home", "Desktop", "Downloads", "Documents", "---", "Local disk (C:)", "Data (D:)"]);
  assert.deepEqual(field.options().map(option => nodes(option).filter(item => item.props?.className === "place-path").map(item => item.props.children)),
    [[page.mock.localPath], [`${page.mock.localPath}\\Desktop`], [`${page.mock.localPath}\\Downloads`], [`${page.mock.localPath}\\Documents`], [], []]);
  // The folder the pane shows is marked, and the highlight starts on it.
  assert.deepEqual(field.options().map(option => option.props["aria-current"]), [true, undefined, undefined, undefined, undefined, undefined]);
  assert.equal(field.selected(), 0);
  assert.equal(field.input().props["aria-activedescendant"], field.options()[0].props.id);
  assert.equal(field.listbox().props.role, "listbox");
  assert.equal(field.input().props["aria-controls"], field.listbox().props.id);
});

test("places without drive letters show the root or mount name and skip the separator when no folder precedes", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  page.mock.setPlaces([{ kind: "home", path: "/home/user", name: null }, { kind: "drive", path: "/", name: null }, { kind: "drive", path: "/Volumes/Backup", name: "Backup" }]);
  field.press("ArrowDown"); await flush();
  assert.deepEqual(field.rows(), ["Home", "---", "/", "Backup"]);
  field.press("Escape");
  page.mock.setPlaces([{ kind: "drive", path: "C:\\", name: "" }]);
  field.press("ArrowDown"); await flush();
  assert.deepEqual(field.rows(), ["Local disk (C:)"]);
});

test("arrow keys move the highlight with wrapping, Home and End, and Enter navigates to the highlighted path", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  field.press("ArrowDown"); await flush();
  assert.equal(field.press("ArrowDown"), true);
  assert.equal(field.press("ArrowDown"), true);
  assert.equal(field.selected(), 2);
  assert.equal(field.input().props["aria-activedescendant"], field.options()[2].props.id);
  assert.equal(field.listbox().props["data-keys"], true);
  field.press("End"); assert.equal(field.selected(), 5);
  field.press("ArrowDown"); assert.equal(field.selected(), 0);
  field.press("ArrowUp"); assert.equal(field.selected(), 5);
  field.press("Home"); assert.equal(field.selected(), 0);
  field.press("ArrowDown"); field.press("ArrowDown");
  assert.equal(field.press("Enter"), true, "Enter picks the place instead of submitting the form");
  await flush();
  assert.deepEqual(page.mock.calls.at(-1), ["localList", `${page.mock.localPath}\\Downloads`]);
  assert.equal(field.listbox(), undefined);
  assert.equal(nodes(page.pane("local").tree).find(item => item.type?.name === "PlacesField").props.value, `${page.mock.localPath}\\Downloads`);
});

test("Escape and Tab close the list, Enter submits the typed path while it is closed, typing closes it", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  assert.equal(field.press("Enter"), false);
  assert.equal(field.press("ArrowUp"), false);
  field.press("ArrowDown", { altKey: true }); await flush();
  assert(field.listbox());
  assert.equal(field.press("Escape"), true);
  assert.equal(field.listbox(), undefined);
  field.press("ArrowDown"); await flush();
  assert.equal(field.press("Tab"), false, "Tab keeps its default focus move");
  assert.equal(field.listbox(), undefined);
  field.press("ArrowDown"); await flush();
  field.input().props.onChange({ target: { value: `${page.mock.localPath}\\typed` } });
  assert.equal(field.listbox(), undefined);
  assert.equal(page.mock.calls.filter(call => call[0] === "localList").length, 1, "typing alone does not navigate");
});

test("pointer hover moves the highlight without the focus ring and a click navigates", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  field.press("ArrowDown"); await flush();
  field.press("ArrowDown");
  assert.equal(field.listbox().props["data-keys"], true);
  field.options()[4].props.onPointerEnter();
  assert.equal(field.selected(), 4);
  assert.equal(field.listbox().props["data-keys"], undefined);
  field.options()[4].props.onClick();
  await flush();
  assert.deepEqual(page.mock.calls.at(-1), ["localList", "C:\\"]);
  assert.equal(field.listbox(), undefined);
  const chevron = nodes(field.render()).find(item => item.props?.className?.includes("sftp-path-toggle"));
  chevron.props.onClick(); await flush();
  assert(field.listbox(), "the chevron opens the list");
  assert.equal(page.mock.calls.filter(call => call[0] === "localPlaces").length, 2, "every opening asks for the drives again");
  nodes(field.render()).find(item => item.props?.className?.includes("sftp-path-toggle")).props.onClick();
  assert.equal(field.listbox(), undefined, "the chevron toggles it closed");
});

test("a failing places lookup reports through the pane error banner and opens nothing", async context => {
  const page = await createPage(context);
  const field = placesField(page);
  page.mock.failPlaces();
  field.press("ArrowDown"); await flush();
  assert.equal(field.listbox(), undefined);
  assert(nodes(page.render()).some(node => node.props?.role === "alert" && nodes(node).some(child => child.props?.children === "Error: places denied")));
  page.mock.setPlaces([]);
});

test("outside pointer presses, resizes and scrolls close the list but presses inside the field do not", async context => {
  const listeners = [];
  const target = { addEventListener(type, handler) { listeners.push({ type, handler }); }, removeEventListener() {} };
  Object.defineProperty(globalThis, "document", { configurable: true, value: target });
  Object.defineProperty(globalThis, "window", { configurable: true, value: target });
  context.after(() => { delete globalThis.document; delete globalThis.window; });
  const page = await createPage(context);
  const field = placesField(page);
  nodes(field.render()).find(item => item.props?.className === "sftp-path-field").props.ref.current = { contains: node => node.inside, querySelector: () => null };
  const open = async () => {
    field.press("ArrowDown"); await flush();
    // Only the render that shows the list registers its listeners.
    page.mock.effects.length = 0; listeners.length = 0;
    field.render();
    for (const effect of page.mock.effects.splice(0)) effect();
    assert.deepEqual(listeners.map(listener => listener.type).sort(), ["pointerdown", "resize", "scroll"]);
    assert(field.listbox());
  };
  const fire = type => listeners.find(listener => listener.type === type).handler;
  await open();
  fire("pointerdown")({ target: { inside: true } });
  assert(field.listbox(), "a press inside the field keeps the list open");
  fire("pointerdown")({ target: {} });
  assert.equal(field.listbox(), undefined);
  await open();
  fire("scroll")({ target: { inside: true } });
  assert(field.listbox(), "scrolling the list itself keeps it open");
  fire("scroll")({ target: {} });
  assert.equal(field.listbox(), undefined);
  await open();
  fire("resize")({});
  assert.equal(field.listbox(), undefined);
});

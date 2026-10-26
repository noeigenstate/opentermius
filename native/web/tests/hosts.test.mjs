import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Hosts.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;

// Run the actual page and its pure functions with hooks and native IPC isolated.
async function createPage(context, storage = new Map()) {
  const originals = ["localStorage", "HTMLButtonElement", "getComputedStyle"].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: {
    getItem: (key) => storage.get(key), setItem: (key, value) => storage.set(key, value),
  } });
  Object.defineProperty(globalThis, "HTMLButtonElement", { configurable: true, value: class {} });
  Object.defineProperty(globalThis, "getComputedStyle", { configurable: true, value: () => ({ gridTemplateColumns: "1fr 1fr 1fr" }) });
  context.after(() => {
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });
  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    let cursor = 0;
    const slots = [];
    export const calls = [];
    export let hosts = [];
    export function setHosts(next) { hosts = next; }
    export function begin() { cursor = 0; }
    export function useState(initial) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    }
    export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
    export function useMemo(compute) { return compute(); }
    export function useEffect() {}
    export function jsx(type, props) { return { type, props }; }
    export const jsxs = jsx;
    export function t(text) { return text; }
    export function Icon() {}
    export function UsageRing() {}
    export function HostEditor() {}
    export function HostModal() {}
    export function Menu() {}
    export async function setHostFavorite(id, favorite) { calls.push({ id, favorite }); }
    export let reorderError;
    export function failReorder(error) { reorderError = error; }
    export async function reorderHosts(ids) { calls.push({ reorder: ids }); if (reorderError) throw reorderError; }
    export async function listHosts() { calls.push("list_hosts"); return hosts; }
    export async function getHost() {}
    export async function deleteHost() {}
    export async function duplicateHost() {}
    export async function renameGroup() {}
    export function newHostDraft(group) { return { group }; }
    export async function parseQuickConnect(text) { calls.push({ quickConnect: text }); return { host: "example.invalid" }; }
  `);
  let compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  compiled = compiled.replace(/^import "\.\.\/hosts\.css";\r?\n/m, "").replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  const page = await import(moduleUrl(compiled));
  const mock = await import(mockUrl);
  return { ...page, mock, render: (props) => { mock.begin(); return page.HostsPage(props); } };
}

function findAll(node, predicate) {
  if (Array.isArray(node)) return node.flatMap((child) => findAll(child, predicate));
  if (!node || typeof node !== "object") return [];
  return [...(predicate(node) ? [node] : []), ...findAll(node.props?.children, predicate)];
}
const byClass = (tree, className) => findAll(tree, (node) => node.props?.className?.split(" ").includes(className));
const host = (id, group = "", favorite = false) => ({ id, name: id, host: "example.invalid", port: 22, username: "fixture", group, tags: [], favorite, lastConnected: null });
const flush = async () => { await new Promise((resolve) => setImmediate(resolve)); };

test("visible subset moves beside its new neighbour and preserves hidden host order", async (context) => {
  const { moveVisibleHost } = await createPage(context);
  const hosts = [host("hidden-0"), host("a"), host("hidden-1"), host("b"), host("hidden-2"), host("c"), host("hidden-3")];
  const ids = (items) => items.map((item) => item.id);
  assert.deepEqual(ids(moveVisibleHost(hosts, ["a", "b", "c"], "a", 2)), ["hidden-0", "hidden-1", "b", "hidden-2", "c", "a", "hidden-3"]);
  assert.deepEqual(ids(moveVisibleHost(hosts, ["a", "b", "c"], "c", 0)), ["hidden-0", "c", "a", "hidden-1", "b", "hidden-2", "hidden-3"]);
  assert.deepEqual(ids(hosts), ["hidden-0", "a", "hidden-1", "b", "hidden-2", "c", "hidden-3"]);
  assert.equal(moveVisibleHost(hosts, ["a", "b", "c"], "c", 0)[1], hosts[5]);
});

test("no-op drops and unavailable hosts return the original order", async (context) => {
  const { moveVisibleHost } = await createPage(context);
  const hosts = [host("a"), host("b")];
  for (const [visible, id, target] of [[["a", "b"], "a", 0], [["a"], "a", 1], [[], "a", 0], [["a", "b"], "missing", 1], [["a", "missing"], "a", 1]]) {
    assert.equal(moveVisibleHost(hosts, visible, id, target), hosts);
  }
});

test("keyboard moves clamp at both ends and move one visible position", async (context) => {
  const { moveVisibleHost } = await createPage(context);
  const hosts = [host("a"), host("hidden"), host("b"), host("c")];
  const visible = ["a", "b", "c"];
  assert.equal(moveVisibleHost(hosts, visible, "a", -1), hosts);
  assert.equal(moveVisibleHost(hosts, visible, "c", 3), hosts);
  assert.deepEqual(moveVisibleHost(hosts, visible, "a", 1).map((item) => item.id), ["hidden", "b", "a", "c"]);
  assert.deepEqual(moveVisibleHost(hosts, visible, "c", 1).map((item) => item.id), ["a", "hidden", "c", "b"]);
});

test("Alt+Arrow reorders optimistically, persists and refreshes without replacing card focus", async (context) => {
  const { render, mock } = await createPage(context);
  const hosts = [host("a"), host("b"), host("c")];
  const updates = [];
  const props = { hosts, onHostsChange: (next) => updates.push(next) };
  let tree = render(props);
  const grid = byClass(tree, "hosts-grid")[0];
  const cards = hosts.map(() => ({}));
  grid.props.ref.current = { children: cards };
  grid.props.children[0].props.onSelect();
  mock.setHosts([hosts[1], hosts[0], hosts[2]]);
  tree = render(props);
  let prevented = false;
  byClass(tree, "hosts-grid")[0].props.onKeyDown({ key: "ArrowRight", altKey: true, target: cards[0], preventDefault: () => { prevented = true; } });
  assert(prevented);
  assert.deepEqual(updates[0].map((item) => item.id), ["b", "a", "c"]);
  await flush();
  assert.deepEqual(mock.calls, [{ reorder: ["b", "a", "c"] }, "list_hosts"]);
  assert.equal(updates.at(-1), mock.hosts);
  const moved = byClass(render({ ...props, hosts: mock.hosts }), "hosts-grid")[0].props.children[1];
  assert(moved.props.selected);
  assert.equal(moved.type(moved.props).props["aria-description"], "Drag or press Alt+Arrow to reorder");
});

test("failed reorder rolls back the full list and shows the existing error banner", async (context) => {
  const { render, mock } = await createPage(context);
  const hosts = [host("a", "", true), host("hidden"), host("b", "", true)];
  const updates = [];
  const props = { hosts, favoritesOnly: true, onHostsChange: (next) => updates.push(next) };
  const grid = byClass(render(props), "hosts-grid")[0];
  const card = {};
  grid.props.ref.current = { children: [card, {}] };
  grid.props.children[0].props.onSelect();
  mock.failReorder("fixture save failure");
  byClass(render(props), "hosts-grid")[0].props.onKeyDown({ key: "ArrowDown", altKey: true, target: card, preventDefault() {} });
  await flush();
  assert.deepEqual(updates[0].map((item) => item.id), ["hidden", "b", "a"]);
  assert.equal(updates.at(-1), hosts);
  assert.deepEqual(mock.calls, [{ reorder: ["hidden", "b", "a"] }]);
  assert.equal(byClass(render(props), "banner-error")[0].props.children[1].props.children, "fixture save failure");
});

test("favorite filtering preserves source data and order, including legacy missing flags", async (context) => {
  const { filterFavorites } = await createPage(context);
  const hosts = [host("a", "Work", true), host("b"), host("c", "Work", true), { ...host("legacy"), favorite: undefined }];
  assert.equal(filterFavorites(hosts, false), hosts);
  assert.deepEqual(filterFavorites(hosts, true), [hosts[0], hosts[2]]);
  assert.equal(filterFavorites(hosts, true)[0], hosts[0]);
  assert.equal(hosts.length, 4);
  assert.deepEqual(filterFavorites([], true), []);
});

test("group counts include ungrouped hosts and follow the favorites scope", async (context) => {
  const { groupCounts, filterFavorites } = await createPage(context);
  const hosts = [host("a", "Work", true), host("b", "Work"), host("c"), host("d", "work"), host("e", "", true)];
  assert.deepEqual([...groupCounts(hosts)], [["Work", 2], ["", 2], ["work", 1]]);
  assert.deepEqual([...groupCounts(filterFavorites(hosts, true))], [["Work", 1], ["", 1]]);
  assert.equal(groupCounts([]).size, 0);
});

test("host tones are stable, bounded, and cover all six tones", async (context) => {
  const { hostTone } = await createPage(context);
  const tones = ["blue", "purple", "green", "orange", "pink", "cyan"];
  assert.equal(hostTone(""), "blue");
  assert.equal(hostTone("a"), "purple");
  assert.equal(hostTone("fixture-0"), hostTone("fixture-0"));
  for (const id of ["中文", "🖥️", "x".repeat(10000)]) assert(tones.includes(hostTone(id)));
  assert.deepEqual(new Set(Array.from({ length: 100 }, (_, index) => hostTone(`fixture-${index}`))), new Set(tones));
});

test("OS icons cover every normalized id and unknown or missing values keep the server glyph", async (context) => {
  const { hostOsIcon } = await createPage(context);
  const icons = {
    ubuntu: ["osUbuntu", "Ubuntu"], debian: ["osDebian", "Debian"], centos: ["osCentos", "CentOS"],
    rhel: ["osRhel", "Red Hat Enterprise Linux"], fedora: ["osFedora", "Fedora"], rocky: ["osRocky", "Rocky Linux"],
    almalinux: ["osAlmalinux", "AlmaLinux"], alpine: ["osAlpine", "Alpine Linux"], arch: ["osArch", "Arch Linux"],
    opensuse: ["osOpensuse", "openSUSE"], amazon: ["osAmazon", "Amazon Linux"], kali: ["osKali", "Kali Linux"],
    raspbian: ["osRaspbian", "Raspbian"], linux: ["osLinux", "Linux"], freebsd: ["osFreebsd", "FreeBSD"],
    macos: ["osMacos", "macOS"], windows: ["osWindows", "Windows"],
  };
  for (const [id, [name, label]] of Object.entries(icons)) {
    const icon = hostOsIcon(id);
    assert.deepEqual({ name: icon.name, label: icon.label }, { name, label });
    assert.match(icon.color, /^#[0-9A-F]{6}$/, `${id} lacks its official brand colour`);
  }
  for (const id of [undefined, null, "", "unknown", "constructor", "__proto__"]) {
    assert.deepEqual(hostOsIcon(id), { name: "hosts", label: "Server" });
  }
});

test("grid and list tiles expose the OS label and preserve size and tone", async (context) => {
  for (const mode of ["grid", "list"]) {
    const { render } = await createPage(context, new Map([["hosts-view", mode]]));
    for (const [os, label, name] of [["ubuntu", "Ubuntu", "osUbuntu"], ["windows", "Windows", "osWindows"], [undefined, "Server", "hosts"], ["unknown", "Server", "hosts"]]) {
      const element = byClass(render({ hosts: [{ ...host("a"), os }] }), `hosts-${mode}`)[0].props.children[0];
      const tile = byClass(element.type(element.props), "host-server-glyph")[0];
      assert.equal(tile.props.role, "img");
      assert.equal(tile.props["aria-label"], label);
      assert.equal(tile.props["data-tooltip"], label);
      assert.equal(tile.props["data-tone"], "purple");
      assert.equal(tile.props.children.props.name, name);
      assert.equal(tile.props.children.props.size, 22);
    }
  }
});

test("page pills show counts and favorites have their own heading and empty state", async (context) => {
  const { render } = await createPage(context);
  const hosts = [host("a", "Work", true), host("b", "Work"), host("c")];
  let tree = render({ hosts });
  assert.deepEqual(byClass(tree, "host-group-count").map((node) => node.props.children), [3, 2, 1]);
  tree = render({ hosts, favoritesOnly: true });
  assert.deepEqual(byClass(tree, "host-group-count").map((node) => node.props.children), [1, 1]);
  assert.equal(findAll(tree, (node) => node.type === "h1")[0].props.children, "Favorites");
  tree = render({ hosts: [], favoritesOnly: true });
  assert.equal(findAll(tree, (node) => node.type === "h2")[0].props.children, "No favorites yet");
});

test("list view persists and ArrowDown moves by one even when CSS reports three columns", async (context) => {
  const storage = new Map();
  const { render } = await createPage(context, storage);
  const props = { hosts: [host("a"), host("b"), host("c")] };
  let tree = render(props);
  findAll(tree, (node) => node.props?.["aria-label"] === "List view")[0].props.onClick();
  tree = render(props);
  assert.equal(storage.get("hosts-view"), "list");
  const list = byClass(tree, "hosts-list")[0];
  let focused;
  list.props.ref.current = { children: props.hosts.map((item) => ({ focus: () => { focused = item.id; } })) };
  list.props.children[0].props.onSelect();
  tree = render(props);
  let prevented = false;
  byClass(tree, "hosts-list")[0].props.onKeyDown({ key: "ArrowDown", target: {}, preventDefault: () => { prevented = true; } });
  assert.equal(focused, "b");
  assert(prevented);
});

test("favorites view preference is independent and unavailable storage is harmless", async (context) => {
  const { render } = await createPage(context, new Map([["favorites-view", "list"], ["hosts-view", "grid"]]));
  assert.equal(byClass(render({ hosts: [host("a", "", true)], favoritesOnly: true }), "hosts-list").length, 1);
  localStorage.setItem = () => { throw Error("storage blocked"); };
  findAll(render({ hosts: [] }), (node) => node.props?.["aria-label"] === "Grid view")[0].props.onClick();
  const second = await createPage(context, { get() { throw Error("storage blocked"); } });
  assert.equal(byClass(second.render({ hosts: [host("a")] }), "hosts-grid").length, 1);
});

test("favorite action persists the inverse flag and refreshes the host collection", async (context) => {
  const { render, mock } = await createPage(context);
  const hosts = [host("a")];
  const updated = [host("a", "", true)];
  mock.setHosts(updated);
  let received;
  const card = byClass(render({ hosts, onHostsChange: (next) => { received = next; } }), "hosts-grid")[0].props.children[0];
  card.props.onFavorite();
  await flush();
  assert.deepEqual(mock.calls, [{ id: "a", favorite: true }, "list_hosts"]);
  assert.equal(received, updated);
});

test("card exposes status, tone and favorite, with SFTP only in the host menu", async (context) => {
  const { render } = await createPage(context);
  const element = byClass(render({ hosts: [host("a", "", true)], onConnect() {}, onSftp() {}, connectionStates: new Map([["a", "connected"]]) }), "hosts-grid")[0].props.children[0];
  const card = element.type(element.props);
  assert.equal(card.props["data-online"], true);
  assert.equal(byClass(card, "host-server-glyph")[0].props["data-tone"], "purple");
  assert.equal(byClass(card, "host-favorite")[0].props["aria-pressed"], true);
  assert.equal(byClass(card, "host-sftp").length, 0);
  assert.equal(byClass(card, "host-more").length, 1);
  assert.equal(byClass(card, "host-card-footer").length, 0);
  assert.equal(byClass(card, "host-usage-note").length, 0);
});

test("search retains ssh quick connect with the new placeholder", async (context) => {
  const { render, mock } = await createPage(context);
  let draft;
  const props = { hosts: [], onQuickConnect: (next) => { draft = next; } };
  const input = () => findAll(render(props), (node) => node.type === "input")[0];
  assert.equal(input().props.placeholder, "Search hosts, IPs or tags…");
  input().props.onChange({ target: { value: "ssh fixture@example.invalid" } });
  input().props.onKeyDown({ key: "Enter", preventDefault() {} });
  await flush();
  assert.equal(draft.host, "example.invalid");
  assert.deepEqual(mock.calls, [{ quickConnect: "ssh fixture@example.invalid" }]);
});

test("host cards render all three connection labels and show usage only while connected", async (context) => {
  const { render } = await createPage(context);
  for (const [connectionState, label] of [[undefined, "SSH not connected"], ["connected", "SSH connected"], ["lost", "Connection lost"]]) {
    const element = byClass(render({ hosts: [host("a")], connectionStates: new Map([["a", connectionState]]), usageByHost: { a: { cpu: 10, memory: 20, disk: 30 } } }), "hosts-grid")[0].props.children[0];
    const card = element.type(element.props);
    const status = byClass(card, "host-connection-state")[0];
    assert.equal(status.props.children[1], label);
    assert.equal(status.props.className.includes("is-connected"), connectionState === "connected");
    assert.equal(byClass(card, "dot-online").length, connectionState === "connected" ? 1 : 0);
    assert.equal(byClass(card, "dot-lost").length, connectionState === "lost" ? 1 : 0);
    assert.deepEqual(byClass(card, "host-metrics")[0].props.children.map(ring => ring.props.value), connectionState === "connected" ? [10, 20, 30] : [null, null, null]);
  }
});

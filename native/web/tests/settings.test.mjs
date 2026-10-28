import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Settings.tsx", import.meta.url), "utf8");
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const transpile = (code, jsx) => ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, ...(jsx && { jsx: ts.JsxEmit.ReactJSX }) } }).outputText;
const { changelog } = await import(moduleUrl(transpile(await readFile(new URL("../src/changelog.ts", import.meta.url), "utf8"))));
globalThis.__changelog = changelog;
const mock = moduleUrl(`
  // Hook values live in numbered slots. Tests that need state call mount() below, which resets the slots and replays effects.
  const hooks = globalThis.__hooks = { slots: [], index: 0, pending: [] };
  export function useState(value) { const i = hooks.index++; if (!(i in hooks.slots)) hooks.slots[i] = value; return [hooks.slots[i], next => { hooks.slots[i] = next; }]; }
  export function useRef(value) { const i = hooks.index++; return hooks.slots[i] ??= { current: value }; }
  export function useEffect(effect, deps) {
    const i = hooks.index++, previous = hooks.slots[i];
    if (previous && deps && deps.every((dep, n) => Object.is(dep, previous.deps[n]))) return;
    hooks.slots[i] = { deps, cleanup: previous?.cleanup };
    hooks.pending.push(() => { hooks.slots[i].cleanup?.(); hooks.slots[i].cleanup = effect(); });
  }
  export function jsx(type, props) { return { type, props }; }
  export const jsxs = jsx, Fragment = "Fragment";
  export function t(text) { return text; }
  export const Icon = "Icon", Recovery = "Recovery", changelog = globalThis.__changelog;
  export async function ask() {}
  export async function open() {}
  export async function save() {}
  export async function getAppInfo() { return globalThis.__info; }
  export async function openConfigFolder() { globalThis.__opened.push("config"); }
  export async function openProjectLink(link) { if (link === globalThis.__failLink) throw new Error("no browser"); globalThis.__opened.push(link); }
  export async function detectXshellFolder() {}
  export async function exportHosts() {}
  export async function importHostsJson() {}
  export async function importWindterm() {}
  export async function importXshell() {}
`);
const compiled = transpile(source, true).replace(/from "[^\"]+"/g, `from ${JSON.stringify(mock)}`);
const { SettingsPage } = await import(moduleUrl(compiled));

function nodes(node) {
  if (!node || typeof node !== "object") return [];
  return [node, ...[node.props?.children].flat(Infinity).flatMap(nodes)];
}

// Walks the tree like nodes(), but also renders function components (Group, Changelog, About, ...).
function expanded(node) {
  if (!node || typeof node !== "object") return [];
  if (typeof node.type === "function") return [node, ...expanded(node.type(node.props))];
  return [node, ...[node.props?.children].flat(Infinity).flatMap(expanded)];
}
const text = node => [node?.props?.children].flat(Infinity).filter(child => typeof child === "string").join("");
const withClass = name => node => node.props?.className?.split(" ").includes(name);
const withView = view => node => node.props?.["data-view"] === view;
const baseSettings = { language: "en", appearance: "light", themeSchedule: false, themeDayStart: "07:00", themeNightStart: "19:00", fontSize: 14, scrollbackLines: 10000, keepaliveSeconds: 30, connectionTimeoutSeconds: 15, autoUpdate: false };

/** Renders SettingsPage with working state: render() runs the component, commit() the effects it queued, show() both. */
function mount(version = "0.8.6") {
  const hooks = Object.assign(globalThis.__hooks, { slots: [], index: 0, pending: [] });
  Object.assign(globalThis, { __info: { version, configFolder: "<config folder>" }, __opened: [], __failLink: null });
  const keys = [];
  globalThis.document = { addEventListener: (_type, handler) => keys.push(handler), removeEventListener: (_type, handler) => keys.splice(keys.indexOf(handler), 1) };
  const ui = {
    keys,
    render() { hooks.index = 0; ui.tree = SettingsPage({ settings: baseSettings, saving: false, onChange: async () => {}, updates: {} }); ui.hooks = hooks.index; },
    commit() { for (const run of hooks.pending.splice(0)) run(); },
    show() { ui.render(); ui.commit(); return ui; },
    // Function components share the hook slots after the page's own, so rewind to the same place before rendering them.
    all() { hooks.index = ui.hooks; return expanded(ui.tree); },
    find: pick => ui.all().find(pick),
    click(pick) { ui.find(pick).props.onClick(); return ui.show(); },
    // Lets getAppInfo and the open_* commands resolve, then draws the page again.
    async settle() { await new Promise(resolve => setTimeout(resolve)); return ui.show(); },
    pressKey(key = "Escape") { const event = { key, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } }; ui.keys.slice().forEach(handler => handler(event)); return event; },
  };
  return ui.show();
}

test("Appearance lists Theme after Language and saves all four options without changing other settings", () => {
  const settings = { language: "en", appearance: "light", themeSchedule: false, themeDayStart: "07:00", themeNightStart: "19:00", fontSize: 14 };
  const writes = [];
  const tree = SettingsPage({ settings, saving: false, onChange: async value => { writes.push(value); }, updates: {} });
  const selects = nodes(tree).filter(node => node.type === "select");
  assert.deepEqual(selects.slice(0, 2).map(node => node.props["aria-label"]), ["Language", "Theme"]);
  const theme = selects[1];
  assert.equal(theme.props.value, "light");
  assert.deepEqual(theme.props.children.map(node => [node.props.value, node.props.children]), [
    ["light", "White background, black text"], ["dark", "Black background, white text"],
    ["system", "Follow system"], ["schedule", "Day/night by time"],
  ]);
  for (const appearance of ["light", "dark", "system", "schedule"]) theme.props.onChange({ target: { value: appearance } });
  assert.deepEqual(writes, ["light", "dark", "system", "schedule"].map(value => ({ ...settings, appearance: value === "schedule" ? settings.appearance : value, themeSchedule: value === "schedule" })));
  assert.equal(nodes(tree).filter(node => node.props?.type === "time").length, 0);
});

test("schedule alone reveals both time inputs and saves their values", () => {
  const settings = { language: "en", appearance: "system", themeSchedule: true, themeDayStart: "07:00", themeNightStart: "19:00" };
  const writes = [];
  const tree = SettingsPage({ settings, saving: false, onChange: async value => { writes.push(value); }, updates: {} });
  assert.equal(nodes(tree).find(node => node.props?.["aria-label"] === "Theme").props.value, "schedule");
  const times = nodes(tree).filter(node => node.props?.type === "time");
  assert.deepEqual(times.map(node => [node.props["aria-label"], node.props.value]), [["Light from", "07:00"], ["Dark from", "19:00"]]);
  times[0].props.onChange({ target: { value: "08:30" } });
  times[1].props.onChange({ target: { value: "20:15" } });
  assert.deepEqual(writes, [{ ...settings, themeDayStart: "08:30" }, { ...settings, themeNightStart: "20:15" }]);
});

test("theme selection preserves the fallback and disables scheduling in one settings update", () => {
  for (const appearance of ["light", "dark", "system"]) {
    for (const themeSchedule of [false, true]) {
      const settings = { appearance, themeSchedule, themeDayStart: "08:30", themeNightStart: "20:15" };
      for (const value of ["light", "dark", "system", "schedule"]) {
        const writes = [];
        const tree = SettingsPage({ settings, saving: false, onChange: async next => { writes.push(next); }, updates: {} });
        const theme = nodes(tree).find(node => node.props?.["aria-label"] === "Theme");
        assert.equal(theme.props.value, themeSchedule ? "schedule" : appearance);
        assert.equal(nodes(tree).filter(node => node.props?.type === "time").length, themeSchedule ? 2 : 0);
        theme.props.onChange({ target: { value } });
        assert.deepEqual(writes, [{ ...settings, appearance: value === "schedule" ? appearance : value, themeSchedule: value === "schedule" }]);
      }
    }
  }
});

test("Chinese theme labels are correct and translation keys are unique across all feature files", async () => {
  const directory = new URL("../src/i18n/zh/", import.meta.url);
  const keys = new Set();
  let labels;
  for (const file of await readdir(directory)) {
    if (!file.endsWith(".ts")) continue;
    const source = await readFile(new URL(file, directory), "utf8");
    const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.ES2022, true);
    function visit(node) {
      if (ts.isPropertyAssignment(node) && ts.isStringLiteral(node.name)) {
        assert(!keys.has(node.name.text), `duplicate translation key: ${node.name.text}`);
        keys.add(node.name.text);
      }
      ts.forEachChild(node, visit);
    }
    visit(parsed);
    if (file === "settings.ts") labels = (await import(moduleUrl(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText))).default;
  }
  for (const [key, value] of Object.entries({ Theme: "主题", "White background, black text": "白底黑字", "Black background, white text": "黑底白字", "Follow system": "跟随系统", "Day/night by time": "昼夜自动切换", "Light from": "浅色开始", "Dark from": "深色开始" })) assert.equal(labels[key], value);
});

test("the About group ends with Changelog and About rows; Version and the update controls stay", () => {
  const ui = mount();
  const group = ui.all().find(node => node.props?.title === "About");
  const children = group.props.children.flat(Infinity).filter(Boolean);
  const rows = children.filter(node => node.type === "button");
  assert.deepEqual(rows.map(row => row.props["data-view"]), ["changelog", "about"]);
  assert.deepEqual(children.slice(-2), rows);
  for (const [row, icon, title, note] of [[rows[0], "list", "Changelog", "Recent improvements and fixes"], [rows[1], "info", "About OpenTermius", "Version, project links and configuration folder"]]) {
    const parts = expanded(row);
    assert(withClass("settings-nav")(row) && withClass("settings-row")(row));
    assert.deepEqual(parts.filter(node => node.type === "Icon").map(node => node.props.name), [icon, "chevronRight"]);
    assert.equal(text(parts.find(node => node.type === "strong")), title);
    assert.equal(text(parts.find(node => node.type === "small")), note);
  }
  const texts = ui.all().map(text);
  for (const kept of ["Version", "Check for updates automatically", "Check now", "Build copies open the release page for manual installation."]) assert(texts.includes(kept), kept);
  assert(!texts.includes("Configuration folder"), "the configuration folder moved to the About page");
});

test("Changelog lists releases newest first and marks the running version; the back button and Esc return", async () => {
  const ui = mount("0.8.6");
  await ui.settle();
  ui.click(withView("changelog"));
  assert.equal(text(ui.find(node => node.type === "h1")), "Changelog");
  assert.equal(ui.find(withView("changelog")), undefined);
  const back = ui.find(withClass("settings-back"));
  assert.deepEqual([expanded(back).find(node => node.type === "Icon").props.name, text(back)], ["chevronLeft", "Settings"]);
  const sections = ui.all().filter(withClass("changelog-release"));
  const versions = sections.map(section => text(expanded(section).find(node => node.type === "strong")));
  assert.deepEqual(versions, changelog.map(release => release.version));
  assert.deepEqual([...versions].sort((a, b) => b.localeCompare(a, undefined, { numeric: true })), versions);
  assert.deepEqual(sections.map(section => expanded(section).filter(node => node.type === "li").map(text)), changelog.map(release => release.items));
  assert.equal(text(expanded(sections[0]).find(node => node.type === "time")), changelog[0].date);
  assert.deepEqual(sections.map(section => expanded(section).some(withClass("chip"))), sections.map((_, index) => index === 0));
  assert.equal(text(ui.all().find(withClass("chip"))), "Current");

  ui.click(withClass("settings-back"));
  assert.equal(text(ui.find(node => node.type === "h1")), "Settings");
  assert(ui.find(withView("changelog")) && ui.find(withView("about")));

  ui.click(withView("changelog"));
  assert.equal(ui.keys.length, 1);
  assert(!ui.pressKey("Enter").defaultPrevented);
  assert.equal(text(ui.show().find(node => node.type === "h1")), "Changelog", "only Esc leaves the sub-view");
  const event = ui.pressKey();
  assert(event.defaultPrevented);
  ui.show();
  assert.equal(text(ui.find(node => node.type === "h1")), "Settings");
  assert.equal(ui.keys.length, 0, "the Esc listener is removed with the sub-view");
});

test("the Current chip follows the running version and is absent until it is known", async () => {
  const chipped = ui => ui.all().filter(withClass("changelog-release")).map(section => expanded(section).some(withClass("chip")));
  const old = mount("0.8.5");
  old.click(withView("changelog"));
  assert.equal(chipped(old).filter(Boolean).length, 0);
  assert.deepEqual(chipped(await old.settle()), changelog.map(release => release.version === "0.8.5"));
  const unknown = mount("9.9.9");
  await unknown.settle();
  assert.equal(chipped(unknown.click(withView("changelog"))).filter(Boolean).length, 0);
});

test("the About page shows the configuration folder and opens it and the three project links", async () => {
  const ui = mount("0.8.6");
  await ui.settle();
  ui.click(withView("about"));
  assert.equal(text(ui.find(node => node.type === "h1")), "About OpenTermius");
  const head = expanded(ui.find(withClass("about-head")));
  assert(head.find(withClass("brand-mark")));
  assert.equal(head.find(node => node.type === "Icon").props.name, "logo");
  assert.equal(text(head.find(node => node.type === "h2")), "OpenTermius");
  assert.equal(text(head.find(withClass("chip"))), "0.8.6");
  assert.equal(text(head.find(node => node.type === "p")), "Local-first SSH and SFTP client");
  const rows = ui.all().filter(withClass("settings-row"));
  assert.deepEqual(rows.map(row => text(expanded(row).find(node => node.type === "span"))), ["Configuration folder", "Project page", "Releases", "Report an issue"]);
  assert.equal(text(expanded(rows[0]).find(node => node.type === "small")), "<config folder>");
  const buttons = ui.all().filter(node => node.type === "button" && withClass("btn")(node) && !withClass("settings-back")(node));
  assert.deepEqual(buttons.map(button => [button.props["aria-label"], text(button)]), [["Open Configuration folder", "Open"], ["Open Project page", "Open"], ["Open Releases", "Open"], ["Open Report an issue", "Open"]]);
  for (const button of buttons) button.props.onClick();
  await ui.settle();
  assert.deepEqual(globalThis.__opened, ["config", "repository", "releases", "issues"]);
  assert.equal(ui.find(withClass("banner-error")), undefined);
});

test("a failing link goes to the page error banner, and Changelog's All releases opens the releases page", async () => {
  const ui = mount();
  ui.click(withView("about"));
  globalThis.__failLink = "issues";
  ui.find(node => node.props?.["aria-label"] === "Open Report an issue").props.onClick();
  await ui.settle();
  assert.match(text(expanded(ui.find(withClass("banner-error"))).find(node => node.type === "span")), /no browser/);
  ui.click(withClass("settings-back"));
  ui.click(withView("changelog"));
  assert.equal(ui.find(withClass("banner-error")), undefined, "navigating clears the previous error");
  assert.equal(text(ui.find(withClass("settings-link"))), "All releases");
  ui.find(withClass("settings-link")).props.onClick();
  await ui.settle();
  assert.deepEqual(globalThis.__opened, ["releases"]);
});

test("opening a sub-view scrolls to the top and focuses its heading; returning focuses the row that opened it", () => {
  const ui = mount();
  const calls = [];
  ui.tree.props.ref.current = { closest: selector => ({ scrollTo: (...position) => calls.push(["scroll", selector, ...position]) }), querySelector: selector => ({ focus: () => calls.push(["focus", selector]) }) };
  ui.find(withView("about")).props.onClick();
  ui.render();
  const heading = ui.find(node => node.type === "h1");
  assert.equal(heading.props.tabIndex, -1);
  heading.props.ref.current = { focus: options => calls.push(["heading", options]) }; // React attaches refs before it runs effects
  ui.commit();
  ui.click(withClass("settings-back"));
  assert.deepEqual(calls, [["scroll", ".content", 0, 0], ["heading", { preventScroll: true }], ["focus", '[data-view="about"]']]);
});

test("every new Settings string has a Chinese entry, and nothing mentions a licence", async () => {
  const directory = new URL("../src/i18n/zh/", import.meta.url);
  const zh = {};
  for (const file of await readdir(directory)) Object.assign(zh, (await import(moduleUrl(transpile(await readFile(new URL(file, directory), "utf8"))))).default);
  const spec = {
    Settings: "设置", Open: "打开", Changelog: "更新日志", "Recent improvements and fixes": "了解最近的版本改进与修复",
    "About OpenTermius": "关于 OpenTermius", "Version, project links and configuration folder": "版本信息、项目链接与配置目录",
    Current: "当前版本", "All releases": "全部版本", "Local-first SSH and SFTP client": "本地优先的 SSH 与 SFTP 客户端",
    "Project page": "项目主页", Releases: "发布页面", "Report an issue": "反馈问题",
  };
  for (const [key, value] of Object.entries(spec)) assert.equal(zh[key], value, key);
  const items = changelog.flatMap(release => release.items);
  assert.equal(items.length, 13);
  for (const item of items) assert.match(zh[item] ?? "", /[一-鿿]/, item);
  for (const file of ["../src/changelog.ts", "../src/i18n/zh/changelog.ts", "../src/pages/Settings.tsx"]) assert.doesNotMatch(await readFile(new URL(file, import.meta.url), "utf8"), /licen[cs]e|许可/i, file);
});

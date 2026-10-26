import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const mockUrl = moduleUrl(`
  export function jsx(type, props) { return { type, props }; }
  export const jsxs = jsx;
  export function t(text) { return text; }
  export const Icon = "Icon";
`);
const modules = [];
for (const path of ["../src/pages/Terminals.tsx", "../src/components/Sidebar.tsx"]) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  modules.push(await import(moduleUrl(compiled)));
}
const [{ TerminalsPage }, { Sidebar }] = modules;

function findAll(node, predicate) {
  if (Array.isArray(node)) return node.flatMap(child => findAll(child, predicate));
  if (!node || typeof node !== "object") return [];
  return [...(predicate(node) ? [node] : []), ...findAll(node.props?.children, predicate)];
}

test("Terminals renders the existing page and empty-state pattern with both actions", () => {
  const calls = [];
  const tree = TerminalsPage({ onLocalTerminal: () => calls.push("local"), onHosts: () => calls.push("hosts") });
  assert.equal(tree.props.className, "page");
  const header = findAll(tree, node => node.props?.className === "page-header")[0];
  assert.equal(findAll(header, node => node.type === "h1")[0].props.children, "Terminals");
  assert.equal(findAll(header, node => node.type === "p")[0].props.children, "Open SSH and local terminal sessions.");
  const empty = findAll(tree, node => node.props?.className === "empty-state")[0];
  const glyph = findAll(empty, node => node.props?.className === "empty-glyph")[0];
  assert.equal(glyph.props.children.props.name, "prompt");
  assert.equal(findAll(empty, node => node.type === "h2")[0].props.children, "No open terminals");
  assert.equal(findAll(empty, node => node.type === "p")[0].props.children, "Connect from Hosts or open a local terminal.");
  const buttons = findAll(empty, node => node.type === "button");
  assert.equal(buttons[0].props.children[1], "Local Terminal");
  assert.equal(buttons[1].props.children, "Go to Hosts");
  buttons.forEach(button => button.props.onClick());
  assert.deepEqual(calls, ["local", "hosts"]);
});

test("sidebar has four primary sections, with Terminals second and seven indicator positions", () => {
  const order = ["hosts", "terminals", "keychain", "snippets", "logs", "favorites", "settings"];
  for (const [index, section] of order.entries()) {
    const calls = [];
    const tree = Sidebar({ section, onSelect: next => calls.push(next) });
    const nav = findAll(tree, node => node.props?.className === "nav-list")[0];
    assert.equal(nav.props["data-index"], index);
    assert.equal(nav.props.children[1].length, 4);
    assert.equal(nav.props.children[2].type, "hr");
    assert.equal(nav.props.children[3].length, 3);
    const items = [...nav.props.children[1], ...nav.props.children[3]];
    assert.deepEqual(items.map(item => item.props.id), order);
    assert.equal(items[1].props.label, "Terminals");
    assert.equal(items[1].props.icon, "prompt");
    const active = items.filter(item => item.props.active).map(item => item.type(item.props));
    assert.equal(active.length, 1);
    assert.equal(active[0].props["aria-current"], "page");
    active[0].props.onClick();
    assert.deepEqual(calls, [section]);
  }
});

test("Chinese catalogs have unique keys and the required terminal management translations", async () => {
  const directory = new URL("../src/i18n/zh/", import.meta.url);
  const entries = new Map();
  for (const file of await readdir(directory)) {
    const source = ts.createSourceFile(file, await readFile(new URL(file, directory), "utf8"), ts.ScriptTarget.Latest);
    function visit(node) {
      if (ts.isPropertyAssignment(node)) {
        const key = node.name.text;
        assert.equal(entries.has(key), false, `Duplicate Chinese key: ${key}`);
        entries.set(key, node.initializer.text);
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
  for (const [key, value] of Object.entries({
    "Terminals": "终端管理",
    "Open SSH and local terminal sessions.": "已打开的 SSH 与本地终端会话。",
    "No open terminals": "暂无打开的终端",
    "Connect from Hosts or open a local terminal.": "从主机页连接服务器，或打开本地终端。",
    "Go to Hosts": "前往主机",
    "Manage your saved hosts.": "管理已保存的主机。",
    "Local Terminal": "本地终端",
  })) assert.equal(entries.get(key), value);
  assert.equal(entries.has("Manage saved connections and open SSH sessions."), false);
});

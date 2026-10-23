import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/components/TitleBar.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const mockUrl = moduleUrl(`
  export const calls = [];
  export const getCurrentWindow = () => ({ minimize: () => calls.push("minimize"), toggleMaximize: () => calls.push("maximize"), close: () => calls.push("close") });
  export function jsx(type, props) { return { type, props }; }
  export const jsxs = jsx;
  export const Fragment = "Fragment";
  export function t(text) { return text; }
  export const Icon = "Icon";
  export const Tabs = "Tabs";
  export const useEffect = () => {};
  export const useRef = () => ({ current: null });
  export const useState = (initial) => [initial, () => {}];
  export const createPortal = () => null;
`);
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
}).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
const { TitleBar } = await import(moduleUrl(compiled));
const tabsSource = await readFile(new URL("../src/components/Tabs.tsx", import.meta.url), "utf8");
const tabsCompiled = ts.transpileModule(tabsSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
}).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
const { Tabs } = await import(moduleUrl(tabsCompiled));
const mock = await import(mockUrl);

test("empty session strip retains dragging, new terminal and all window controls", () => {
  let opened = 0;
  const bar = TitleBar({ tabs: { sessions: [], onNew: () => opened++ } });
  const tabs = Tabs(bar.props.children[0].props.children.props);
  assert.equal(tabs.props.children[0], false);
  const action = tabs.props.children[1];
  assert.equal(action.type, "button");
  assert.equal(action.props["aria-label"], "Open local terminal");
  action.props.onClick();
  assert.equal(opened, 1);
  assert.equal(bar.props.children[1].props["data-tauri-drag-region"], true);
  bar.props.children.slice(2).forEach((button) => button.props.onClick());
  assert.deepEqual(mock.calls, ["minimize", "maximize", "close"]);
});

test("session strip passes every existing tab handler through unchanged", () => {
  const tabs = { sessions: [{ sessionId: "fixture" }], activeSessionId: "fixture", onSelect() {}, onNew() {}, onReconnect() {}, onDuplicate() {}, onClose() {} };
  const child = TitleBar({ tabs }).props.children[0].props.children;
  assert.equal(child.type, "Tabs");
  assert.deepEqual(child.props, tabs);
  assert.equal(Tabs(child.props).props.children[0].props.role, "tablist");
});

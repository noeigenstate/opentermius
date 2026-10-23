import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Session.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;

async function createPage() {
  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    let cursor = 0;
    const slots = [];
    export function begin() { cursor = 0; }
    export function useState(initial) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], value => { slots[index] = value; }];
    }
    export function jsx(type, props) { return { type, props }; }
    export const jsxs = jsx;
    export const lazy = () => "SftpView";
    export const Suspense = "Suspense";
    export const SessionBar = "SessionBar";
    export const TerminalView = "TerminalView";
    export function t(text) { return text; }
  `);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  const { SessionPage } = await import(moduleUrl(compiled));
  const mock = await import(mockUrl);
  return (props) => { mock.begin(); return SessionPage({ session: { sessionId: "fixture" }, visible: true, blocked: false, onReconnect() {}, ...props }); };
}

test("SFTP initial view opens its lazy panel immediately and keeps terminal input hidden", async () => {
  const render = await createPage();
  let page = render({ initialView: "sftp" });
  assert.equal(page.props.children[0].props.view, "sftp");
  assert.equal(page.props.children[1].props.visible, false);
  assert.equal(page.props.children[2].props.hidden, false);
  page.props.children[0].props.onViewChange("terminal");
  page = render({ initialView: "sftp" });
  assert.equal(page.props.children[0].props.view, "terminal");
  assert.equal(page.props.children[1].props.visible, true);
  assert.equal(page.props.children[2].props.hidden, true);
});

test("regular sessions start in terminal and retain SFTP after switching back", async () => {
  const render = await createPage();
  let page = render({});
  assert.equal(page.props.children[0].props.view, "terminal");
  assert.equal(page.props.children[2], false);
  page.props.children[0].props.onViewChange("sftp");
  page = render({});
  assert.equal(page.props.children[2].props.hidden, false);
  page.props.children[0].props.onViewChange("terminal");
  page = render({ visible: false });
  assert.equal(page.props.hidden, true);
  assert.equal(page.props.children[2].props.hidden, true);
});

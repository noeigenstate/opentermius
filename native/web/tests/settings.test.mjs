import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/pages/Settings.tsx", import.meta.url), "utf8");
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const mock = moduleUrl(`
  export function useState(value) { return [value, () => {}]; }
  export function useEffect() {}
  export function jsx(type, props) { return { type, props }; }
  export const jsxs = jsx, Fragment = "Fragment";
  export function t(text) { return text; }
  export const Icon = "Icon", Recovery = "Recovery";
  export async function ask() {}
  export async function open() {}
  export async function save() {}
  export async function getAppInfo() {}
  export async function openConfigFolder() {}
  export async function detectXshellFolder() {}
  export async function exportHosts() {}
  export async function importHostsJson() {}
  export async function importWindterm() {}
  export async function importXshell() {}
`);
const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX } }).outputText
  .replace(/from "[^\"]+"/g, `from ${JSON.stringify(mock)}`);
const { SettingsPage } = await import(moduleUrl(compiled));

function nodes(node) {
  if (!node || typeof node !== "object") return [];
  return [node, ...[node.props?.children].flat(Infinity).flatMap(nodes)];
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

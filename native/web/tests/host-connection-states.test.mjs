import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
let fixture = 0;

// Exercise App's real callbacks and effects with only hooks, components and IPC isolated.
async function createApp() {
  const mockUrl = moduleUrl(`
    // fixture ${++fixture}
    let cursor = 0;
    const slots = [];
    const effects = [];
    const listeners = new Set();
    const statuses = new Map();
    let sessions = [];
    let connectError;
    export function begin() { cursor = 0; }
    export function useState(initial) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], value => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    }
    export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
    export function useEffect(effect, dependencies) {
      const index = cursor++;
      if (!slots[index] || dependencies.some((value, i) => value !== slots[index][i])) effects.push(effect);
      slots[index] = dependencies;
    }
    export function runEffects() { for (const effect of effects.splice(0)) effect(); }
    export function jsx(type, props) { return { type, props }; }
    export const jsxs = jsx;
    export const lazy = () => "SessionPage";
    export const Suspense = "Suspense";
    export const HostsPage = "HostsPage", TitleBar = "TitleBar", Sidebar = "Sidebar";
    export const SettingsPage = "SettingsPage", KeychainPage = "KeychainPage", SnippetsPage = "SnippetsPage", LogsPage = "LogsPage";
    export const PromptDialog = "PromptDialog", SurfaceEffects = "SurfaceEffects", Icon = "Icon";
    export function t(text) { return text; }
    export function setLanguage() {}
    export function useUpdates() { return {}; }
    export async function getSettings() { return { language: "en" }; }
    export async function saveSettings(next) { return next; }
    export async function onSettingsChanged() { return () => {}; }
    let hosts = [];
    export function setHosts(next) { hosts = next; }
    export async function listHosts() { return hosts; }
    export async function startSessionEvents() {}
    export function subscribeSessions(listener) { listeners.add(listener); return () => listeners.delete(listener); }
    export function emit(event) {
      if (event.type === "status") statuses.set(event.sessionId, event);
      for (const listener of listeners) listener(event);
    }
    export function latestSessionStatus(id) { return statuses.get(id); }
    export function forgetSession(id) { statuses.delete(id); }
    export function setSessions(next) { sessions = next; }
    export function failConnect(error) { connectError = error; }
    export async function listSessions() { return sessions; }
    export async function connectHost() { if (connectError) throw connectError; return sessions.at(-1)?.sessionId; }
    export async function closeSession(id) { sessions = sessions.filter(session => session.sessionId !== id); }
    export async function reconnectSession() {}
    export async function duplicateSession() {}
    export async function openLocalTerminal() {}
    export async function sessionInput() {}
  `);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replace(/from "[^"]+"/g, `from ${JSON.stringify(mockUrl)}`);
  const app = await import(moduleUrl(compiled));
  const mock = await import(mockUrl);
  return { ...app, mock, render: () => { mock.begin(); const tree = app.default(); mock.runEffects(); return tree; } };
}

function find(node, type) {
  if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
  if (!node || typeof node !== "object") return;
  return node.type === type ? node : find(node.props?.children, type);
}
const session = (values = {}) => ({ sessionId: "session-fixture", hostId: "host-fixture", title: "Fixture", local: false, connected: false, ended: false, ...values });
const flush = async () => { await new Promise(resolve => setImmediate(resolve)); };
const state = (app) => find(app.render(), "HostsPage").props.connectionStates.get("host-fixture");

test("App refreshes host metadata through the existing session status event", async () => {
  const app = await createApp();
  app.render();
  await flush();
  const updated = [{ id: "host-fixture", os: "ubuntu" }];
  app.mock.setHosts(updated);
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: true, ended: false, error: false, message: "", hostsChanged: true });
  await flush();
  assert.equal(find(app.render(), "HostsPage").props.hosts, updated);
});

test("no session means not connected; local terminals do not set host state", async () => {
  const { hostConnectionStates } = await createApp();
  assert.equal(hostConnectionStates([], new Set()).get("host-fixture"), undefined);
  assert.equal(hostConnectionStates([session({ local: true, connected: true }), session({ hostId: "", ended: true })], new Set()).size, 0);
});

test("connected sessions set connected; connecting sessions are not lost", async () => {
  const { hostConnectionStates } = await createApp();
  assert.equal(hostConnectionStates([session({ connected: true })], new Set()).get("host-fixture"), "connected");
  assert.equal(hostConnectionStates([session()], new Set()).get("host-fixture"), undefined);
});

test("ended and native error sessions set lost", async () => {
  const { hostConnectionStates } = await createApp();
  for (const error of [false, true]) {
    assert.equal(hostConnectionStates([session({ ended: true, error })], new Set()).get("host-fixture"), "lost");
  }
});

test("a connected session wins over old ended sessions and connect failures in either order", async () => {
  const { hostConnectionStates } = await createApp();
  const connected = session({ connected: true });
  const ended = session({ sessionId: "old-fixture", ended: true });
  const failed = new Set(["host-fixture"]);
  for (const sessions of [[connected, ended], [ended, connected]]) {
    assert.equal(hostConnectionStates(sessions, failed).get("host-fixture"), "connected");
  }
  assert.deepEqual([...failed], ["host-fixture"], "deriving state must not mutate the failure set");
});

test("connect failure without a session sets lost and clearing it restores the default", async () => {
  const { hostConnectionStates } = await createApp();
  assert.equal(hostConnectionStates([], new Set(["host-fixture"])).get("host-fixture"), "lost");
  assert.equal(hostConnectionStates([], new Set()).get("host-fixture"), undefined);
});

test("App tracks thrown connect failures per host and clears them after success and tab close", async () => {
  const app = await createApp();
  app.render();
  await flush();
  app.mock.failConnect(Error("Fixture failure"));
  find(app.render(), "HostsPage").props.onConnect("host-fixture");
  await flush();
  assert.equal(state(app), "lost");
  assert.equal(find(app.render(), "HostsPage").props.connectionStates.has("other-fixture"), false);
  app.mock.failConnect(undefined);
  app.mock.setSessions([session()]);
  find(app.render(), "HostsPage").props.onConnect("host-fixture");
  await flush();
  assert.equal(state(app), undefined, "a new handshake must not retain a previous command failure");
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: true, ended: false, error: false, message: "" });
  assert.equal(state(app), "connected");
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: false, ended: false, error: false, message: "" });
  assert.equal(state(app), undefined, "a successful connection must clear the prior failure");
  find(app.render(), "TitleBar").props.tabs.onClose("session-fixture");
  await flush();
  assert.equal(state(app), undefined);
});

test("App clears a thrown failure when an existing ended tab is explicitly closed", async () => {
  const app = await createApp();
  app.mock.setSessions([session({ ended: true })]);
  app.render();
  await flush();
  app.mock.failConnect(Error("Fixture failure"));
  find(app.render(), "HostsPage").props.onConnect("host-fixture");
  await flush();
  assert.equal(state(app), "lost");
  // The Hosts page has no tab strip; close the tab from another page.
  find(app.render(), "Sidebar").props.onSelect("logs");
  find(app.render(), "TitleBar").props.tabs.onClose("session-fixture");
  await flush();
  find(app.render(), "Sidebar").props.onSelect("hosts");
  assert.equal(state(app), undefined);
});

test("App clears a thrown host failure when another session successfully reconnects", async () => {
  const app = await createApp();
  app.mock.setSessions([session({ ended: true })]);
  app.render();
  await flush();
  app.mock.failConnect(Error("Fixture failure"));
  find(app.render(), "HostsPage").props.onConnect("host-fixture");
  await flush();
  assert.equal(state(app), "lost");
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: true, ended: false, error: false, message: "" });
  assert.equal(state(app), "connected");
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: false, ended: false, error: false, message: "" });
  assert.equal(state(app), undefined);
});

test("App replays early failures, keeps reconnecting at default, and retains dropped tabs until explicit close", async () => {
  const app = await createApp();
  app.render();
  await flush();
  app.mock.setSessions([session()]);
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: false, ended: true, error: true, message: "Fixture failure" });
  find(app.render(), "HostsPage").props.onConnect("host-fixture");
  await flush();
  assert.equal(state(app), "lost");
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: false, ended: false, error: false, message: "" });
  assert.equal(state(app), undefined);
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: true, ended: false, error: false, message: "" });
  assert.equal(state(app), "connected");
  app.mock.emit({ type: "closed", sessionId: "session-fixture" });
  assert.equal(state(app), "lost");
  assert.equal(find(app.render(), "TitleBar").props.tabs.sessions.length, 1);
  find(app.render(), "TitleBar").props.tabs.onClose("session-fixture");
  await flush();
  assert.equal(state(app), undefined);
});

test("the Hosts page hides tabs and enters a connected host's open session", async () => {
  const app = await createApp();
  app.render();
  await flush();
  const hostsPage = () => find(app.render(), "HostsPage").props;
  const tabs = () => find(app.render(), "TitleBar").props.tabs;
  assert.equal(tabs(), undefined);
  app.mock.setSessions([session()]);
  hostsPage().onConnect("host-fixture");
  await flush();
  app.mock.emit({ type: "status", sessionId: "session-fixture", connected: true, ended: false, error: false, message: "" });
  assert.equal(tabs().activeSessionId, "session-fixture");
  find(app.render(), "Sidebar").props.onSelect("favorites");
  assert.equal(tabs(), undefined);
  find(app.render(), "Sidebar").props.onSelect("logs");
  assert.equal(tabs().activeSessionId, null, "other pages keep the tab strip");
  find(app.render(), "Sidebar").props.onSelect("hosts");
  hostsPage().onOpenSession("host-fixture");
  assert.equal(tabs().activeSessionId, "session-fixture");
  assert.equal(tabs().sessions.length, 1, "entering a connected host must not open a second session");
});

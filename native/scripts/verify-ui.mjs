import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const scriptPath = fileURLToPath(import.meta.url);
const fixtureNames = ['Production', 'API Server', 'Development', 'Database'].sort();
const timeoutMs = 8000;

function loopbackUrl(value, protocols, port) {
  const url = new URL(value);
  assert(protocols.includes(url.protocol), 'Unexpected debugging protocol.');
  assert(['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname), 'Debugging must stay on loopback.');
  assert.equal(url.port, String(port), 'Debugging endpoint changed ports.');
  assert(!url.username && !url.password, 'Debugging endpoint must not contain credentials.');
  return url;
}

async function connect(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`, {
    signal: AbortSignal.timeout(timeoutMs), redirect: 'error',
  });
  assert(response.ok, `Debugging discovery returned HTTP ${response.status}.`);
  const targets = await response.json();
  const pages = targets.filter(target => target.type === 'page' && target.title === 'OpenTermius');
  assert.equal(pages.length, 1, 'Expected exactly one OpenTermius WebView debugging target.');
  const address = loopbackUrl(pages[0].webSocketDebuggerUrl, ['ws:'], port);
  const socket = new WebSocket(address);
  await new Promise((accept, reject) => {
    const timer = setTimeout(() => { socket.close(); reject(new Error('CDP connection timed out.')); }, timeoutMs);
    socket.addEventListener('open', () => { clearTimeout(timer); accept(); }, { once: true });
    socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('CDP connection failed.')); }, { once: true });
  });

  let sequence = 0;
  const pending = new Map();
  const exceptions = [];
  socket.addEventListener('message', event => {
    const message = JSON.parse(String(event.data));
    if (message.method === 'Runtime.exceptionThrown') {
      // Report counts, not arbitrary exception payloads that might contain user data.
      exceptions.push(message.params.exceptionDetails.text);
    }
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(`${request.method}: ${message.error.message}`));
    else request.accept(message.result);
  });
  socket.addEventListener('close', () => {
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(new Error('CDP connection closed.'));
    }
    pending.clear();
  });

  const send = (method, params = {}) => new Promise((accept, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out.`)); }, timeoutMs);
    pending.set(id, { accept, reject, timer, method });
    try { socket.send(JSON.stringify({ id, method, params })); }
    catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
  });
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    assert(!result.exceptionDetails, 'A DOM inspection failed inside the WebView.');
    return result.result.value;
  };
  return { send, evaluate, exceptions, close: () => socket.close() };
}

/** Smoke-test an isolated real Tauri WebView. All interaction uses CDP input.
 * Read only native window bounds; never invoke connection or persistence commands,
 * manufacture app state, or connect fixture hosts. A temporary local terminal
 * exercises the tabs and is closed before capturing the workspace. */
export async function verifyUi({ port = 9337, output }) {
  assert(Number.isInteger(port) && port > 0 && port <= 65535, 'Invalid debugging port.');
  assert(output, '--output is required. Use an ignored test-output directory on E:.');
  const outputDir = resolve(output);
  assert(!/^c:[\\/]/i.test(outputDir), 'Screenshots and reports must stay off C:.');
  await mkdir(outputDir, { recursive: true });
  const started = performance.now();
  const report = {
    schema: 1, platform: process.platform, node: process.version,
    startedAt: new Date().toISOString(), checks: [], screenshots: [],
  };
  let cdp;
  let fixtureVerified = false;
  let metricsOverridden = false;
  let mediaOverridden = false;
  let focusOverridden = false;

  async function check(name, action) {
    const start = performance.now();
    try {
      const details = await action();
      const status = details?.skip ? 'skipped' : 'passed';
      report.checks.push({ name, status, durationMs: Math.round(performance.now() - start), details });
      console.log(`${status.toUpperCase()} ${name}`);
      return details;
    } catch (error) {
      report.checks.push({ name, status: 'failed', durationMs: Math.round(performance.now() - start), error: error.message });
      console.error(`FAILED ${name}: ${error.message}`);
      throw error;
    }
  }

  async function waitFor(expression, message, duration = timeoutMs) {
    const end = performance.now() + duration;
    while (performance.now() < end) {
      if (await cdp.evaluate(expression)) return;
      await delay(80);
    }
    throw new Error(message);
  }

  async function point(selector, { index = 0, text } = {}) {
    const result = await cdp.evaluate(`(() => {
      const elements = [...document.querySelectorAll(${JSON.stringify(selector)})].filter(element =>
        element.getClientRects().length && !element.closest('[hidden]') &&
        (${JSON.stringify(text ?? null)} === null || element.textContent.trim() === ${JSON.stringify(text ?? null)}));
      const element = elements[${index}];
      if (!element) return null;
      element.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' });
      const rect = element.getBoundingClientRect();
      const x = rect.left + rect.width / 2;
      const y = rect.top + rect.height / 2;
      return { x, y, visible: x >= 0 && y >= 0 && x < innerWidth && y < innerHeight &&
        element.contains(document.elementFromPoint(x, y)) };
    })()`);
    assert(result?.visible, `Control is unavailable or covered: ${selector}.`);
    return { x: result.x, y: result.y };
  }

  const move = position => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', button: 'none', buttons: 0, ...position });
  async function click(selector, options = {}) {
    const position = await point(selector, options);
    const button = options.button ?? 'left';
    await move(position);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', button, buttons: button === 'right' ? 2 : 1, clickCount: 1, ...position });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', button, buttons: 0, clickCount: 1, ...position });
  }

  async function key(name) {
    const code = { Escape: 27, Tab: 9 }[name];
    assert(code, 'Unsupported smoke-test key.');
    await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code });
  }

  async function capture(file) {
    assert(fixtureVerified, 'Refusing screenshots before the isolated fixture profile is verified.');
    const { data } = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true, captureBeyondViewport: false });
    const bytes = Buffer.from(data, 'base64');
    assert(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Invalid PNG from the WebView.');
    await writeFile(resolve(outputDir, file), bytes);
    report.screenshots.push({ file, width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) });
  }

  async function animations() {
    return cdp.evaluate(`(() => {
      const animations = document.documentElement.getAnimations({ subtree: true });
      return { count: animations.length,
        infinite: animations.filter(animation => animation.effect?.getTiming().iterations === Infinity).length,
        running: animations.filter(animation => animation.playState === 'running').length };
    })()`);
  }

  async function heapMetrics() {
    try {
      const { metrics } = await cdp.send('Performance.getMetrics');
      const names = ['JSHeapUsedSize', 'JSHeapTotalSize', 'Nodes', 'Documents', 'LayoutCount', 'RecalcStyleCount', 'TaskDuration', 'ScriptDuration'];
      return Object.fromEntries(metrics.filter(metric => names.includes(metric.name)).map(metric => [metric.name, metric.value]));
    } catch { return { unavailable: true }; }
  }

  async function navigate(index, pageSelector, file) {
    const label = await cdp.evaluate(`document.querySelectorAll('.sidebar .nav-item')[${index}]?.textContent.trim()`);
    assert(label, 'Navigation item is missing.');
    await click('.sidebar .nav-item', { index });
    await waitFor(`document.querySelector(${JSON.stringify(pageSelector)})?.textContent.trim() === ${JSON.stringify(label)}`, 'Navigation did not show the expected page.');
    await delay(450);
    assert.equal(await cdp.evaluate(`document.querySelectorAll('main .feature-error, main .settings-error').length`), 0, 'Page reported an error.');
    if (file) await capture(file);
    return { headingMatchesNavigation: true };
  }

  try {
    cdp = await connect(port);
    await check('isolated-fixture-profile', async () => {
      await waitFor(`document.querySelectorAll('.host-card').length >= 4`, 'The fixture host page did not become ready.');
      const fixture = await cdp.evaluate(`(() => {
        const cards = [...document.querySelectorAll('.host-card')];
        return { count: cards.length,
          namesMatch: JSON.stringify(cards.map(card => card.querySelector('h3')?.textContent.trim()).sort()) === ${JSON.stringify(JSON.stringify(fixtureNames))},
          endpointsMatch: cards.every(card => card.querySelector('.host-endpoint > span:first-child')?.textContent.trim() === '<user>@<host>'),
          activeSessions: document.querySelectorAll('.session-tab').length,
          visible: cards.every(card => card.getClientRects().length > 0 && !card.closest('[hidden]')) };
      })()`);
      // Never include unexpected host labels or endpoint text in errors or artifacts.
      assert.equal(fixture.count, 4, 'Refusing a profile without exactly four fixture hosts.');
      assert(fixture.namesMatch && fixture.endpointsMatch && fixture.visible, 'Refusing a profile that does not match the synthetic fixture.');
      assert.equal(fixture.activeSessions, 0, 'Refusing a profile with existing sessions.');
      fixtureVerified = true;
      return fixture;
    });

    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    try { await cdp.send('Performance.enable'); } catch { /* Optional on embedded WebViews. */ }
    try {
      const version = await cdp.send('Browser.getVersion');
      report.browser = { product: version.product, protocolVersion: version.protocolVersion };
    } catch { report.browser = { unavailable: true }; }
    await cdp.send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 255, g: 255, b: 255, a: 1 } });
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1240, height: 800, deviceScaleFactor: 1, mobile: false });
    metricsOverridden = true;
    const originalPreferences = await cdp.evaluate(`({ hasFocus: document.hasFocus(),
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches })`);
    await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }] });
    mediaOverridden = true;
    await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: true });
    focusOverridden = true;
    await move({ x: 4, y: 4 });
    await delay(800);
    report.runtime = await cdp.evaluate(`({ width: innerWidth, height: innerHeight, devicePixelRatio,
      hasFocus: document.hasFocus(), visibility: document.visibilityState,
      motionPaused: document.documentElement.dataset.motionPaused,
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      fineHover: matchMedia('(hover: hover) and (pointer: fine)').matches })`);
    report.runtime.documentFocusBeforeEmulation = originalPreferences.hasFocus;
    report.runtime.systemReducedMotion = originalPreferences.reducedMotion;
    report.runtime.motionPreferenceEmulatedForInteraction = 'no-preference';
    report.runtime.focusEmulatedForInteraction = true;

    await check('full-height-sidebar-and-empty-session-strip', async () => {
      const shell = await cdp.evaluate(`(() => {
        const sidebar = document.querySelector('.sidebar').getBoundingClientRect();
        const titlebar = document.querySelector('.titlebar').getBoundingClientRect();
        const content = document.querySelector('main.content').getBoundingClientRect();
        const inset = parseFloat(getComputedStyle(document.querySelector('.app')).borderTopWidth);
        return { sidebarTop: sidebar.top, sidebarBottom: sidebar.bottom, sidebarRight: sidebar.right,
          titlebarTop: titlebar.top, titlebarLeft: titlebar.left, titlebarBottom: titlebar.bottom,
          contentTop: content.top, viewportHeight: innerHeight, inset,
          sessionTabs: document.querySelectorAll('.session-tabs').length,
          dragRegion: !!document.querySelector('.workspace-shell .titlebar [data-tauri-drag-region]'),
          newTerminal: !!document.querySelector('.titlebar-tabs > .new-terminal') };
      })()`);
      assert(Math.abs(shell.sidebarTop - shell.inset) < 1 && Math.abs(shell.sidebarBottom - shell.viewportHeight + shell.inset) < 1, 'Sidebar does not span the window height inside its outline.');
      assert(Math.abs(shell.titlebarTop - shell.inset) < 1 && Math.abs(shell.titlebarLeft - shell.sidebarRight) < 1, 'Title strip does not start to the right of the sidebar.');
      assert(Math.abs(shell.contentTop - shell.titlebarBottom) < 1, 'Content does not follow the title strip.');
      assert.equal(shell.sessionTabs, 0, 'Session tabs must be absent before a session is opened.');
      assert(shell.dragRegion && shell.newTerminal, 'Empty strip lost dragging or the new-terminal action.');
      return shell;
    });
    await check('single-native-frame-and-aligned-tab-action', async () => {
      await click('.new-terminal');
      await waitFor(`document.querySelectorAll('.session-tab').length === 1`, 'New terminal did not create a session tab.');
      await click('.sidebar .nav-item', { index: 0 });
      await waitFor(`!document.querySelector('#hosts-panel').hidden`, 'Hosts navigation did not restore the hosts page.');
      await delay(220);
      const frame = await cdp.evaluate(`(async () => {
        const inner = await window.__TAURI_INTERNALS__.invoke('plugin:window|inner_size', { label: 'main' });
        const outer = await window.__TAURI_INTERNALS__.invoke('plugin:window|outer_size', { label: 'main' });
        const scale = await window.__TAURI_INTERNALS__.invoke('plugin:window|scale_factor', { label: 'main' });
        const app = document.querySelector('.app');
        const style = getComputedStyle(app);
        const workspace = getComputedStyle(document.querySelector('.workspace-shell'));
        const bounds = app.getBoundingClientRect();
        const tab = document.querySelector('.session-tabs > .session-tab:last-child').getBoundingClientRect();
        const plus = document.querySelector('.new-terminal');
        const action = plus.getBoundingClientRect();
        return { inner, outer, border: style.borderTopWidth, radius: style.borderRadius,
          shape: document.documentElement.dataset.windowShape, scale,
          workspaceBorder: workspace.borderTopWidth, workspaceRadius: workspace.borderRadius,
          background: style.backgroundColor, overflow: style.overflow, borderColor: style.borderTopColor,
          fillsViewport: Math.abs(bounds.width - innerWidth) < 1 && Math.abs(bounds.height - innerHeight) < 1,
          centerOffset: Math.abs(tab.y + tab.height / 2 - action.y - action.height / 2),
          gap: action.x - tab.right, sameContainer: plus.parentElement.matches('.titlebar-tabs'),
          svgIcon: !!plus.querySelector('svg'), viewportWidth: innerWidth };
      })()`);
      assert(['round', 'square'].includes(frame.shape), 'Native window shape was not published.');
      if (frame.shape === 'round') {
        // Chromium snaps the hairline at native DPI, including under CDP metrics emulation.
        assert(Math.abs(parseFloat(frame.border) * frame.scale - Math.floor(frame.scale)) < .01, 'Expected one hairline window outline.');
        assert.equal(frame.radius, '10px');
        assert.equal(frame.background, 'rgb(243, 243, 243)');
        assert.equal(frame.overflow, 'hidden');
        assert.equal(frame.borderColor, 'rgb(217, 217, 217)');
      } else {
        assert.equal(frame.border, '0px');
        assert.equal(frame.radius, '0px');
      }
      assert.equal(frame.workspaceBorder, '0px', 'The inner workspace adds a second window outline.');
      assert.equal(frame.workspaceRadius, '0px', 'The inner workspace adds a second rounded contour.');
      assert(frame.fillsViewport, 'The app leaves an inset around the native client area.');
      assert.equal(frame.inner.width, frame.outer.width, 'Native shadow leaves a horizontal frame.');
      assert.equal(frame.inner.height, frame.outer.height, 'Native shadow leaves a vertical frame.');
      assert(frame.centerOffset < 1 && frame.gap >= 0 && frame.gap <= 10, 'New terminal action is not aligned with the tabs.');
      assert(frame.sameContainer && frame.svgIcon, 'New terminal action is outside the tab layout or uses a text glyph.');
      await check('session-tab-icons-on-midline', async () => {
        const geometry = await cdp.evaluate(`(() => {
          const tab = document.querySelector('.session-tab');
          const centre = element => { const rect = element.getBoundingClientRect(); return rect.top + rect.height / 2; };
          const middle = centre(tab);
          const parts = [['status', tab.querySelector('.tab-icon svg, .tab-icon .dot')], ['menu', tab.querySelector('.tab-action:not(.tab-close) svg')],
            ['close', tab.querySelector('.tab-close svg')], ['title', tab.querySelector('.session-tab-title')]];
          const actions = [...tab.querySelectorAll('.tab-action')];
          return { offsets: Object.fromEntries(parts.map(([name, element]) => [name, element ? Math.abs(centre(element) - middle) : null])),
            sizes: [...tab.querySelectorAll('.tab-action svg')].map(svg => { const rect = svg.getBoundingClientRect(); return [rect.width, rect.height]; }),
            textGlyphs: actions.filter(action => [...action.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim())).length };
        })()`);
        for (const [name, offset] of Object.entries(geometry.offsets)) assert(offset !== null && offset < 0.5, `The session tab ${name} is ${offset}px off the tab midline.`);
        assert.equal(geometry.sizes.length, 2, 'The session tab must show a menu and a close action.');
        assert(geometry.sizes.every(([width, height]) => width === geometry.sizes[0][0] && height === geometry.sizes[0][1]), 'The tab actions use different icon sizes.');
        assert.equal(geometry.textGlyphs, 0, 'The tab actions must be SVG icons, not text glyphs.');
        return geometry;
      });
      await click('.session-tab .tab-close');
      await waitFor(`document.querySelectorAll('.session-tabs').length === 0`, 'Closing the final session did not restore the empty strip.');
      await capture('window-frame.png');
      return frame;
    });
    await check('native-maximize-and-restore', async () => {
      const maximized = () => cdp.evaluate(`window.__TAURI_INTERNALS__.invoke('plugin:window|is_maximized', { label: 'main' })`);
      const restoredShape = await cdp.evaluate(`document.documentElement.dataset.windowShape`);
      assert.equal(await maximized(), false, 'Window must start restored.');
      try {
        await click('.titlebar .chrome', { index: 1 });
        await waitFor(`window.__TAURI_INTERNALS__.invoke('plugin:window|is_maximized', { label: 'main' })`, 'Window did not maximize.');
        assert.equal(await maximized(), true);
        await waitFor(`document.documentElement.dataset.windowShape === 'square'`, 'Maximize did not publish a square contour.');
        const style = await cdp.evaluate(`(() => { const style = getComputedStyle(document.querySelector('.app')); return { border: style.borderTopWidth, radius: style.borderRadius }; })()`);
        assert.deepEqual(style, { border: '0px', radius: '0px' });
      } finally {
        if (await maximized()) await click('.titlebar .chrome', { index: 1 });
        await waitFor(`window.__TAURI_INTERNALS__.invoke('plugin:window|is_maximized', { label: 'main' }).then(value => !value)`, 'Window did not restore.');
        await waitFor(`document.documentElement.dataset.windowShape === ${JSON.stringify(restoredShape)}`, 'Restore did not republish the restored contour.');
      }
      const bounds = await cdp.evaluate(`(async () => ({
        inner: await window.__TAURI_INTERNALS__.invoke('plugin:window|inner_size', { label: 'main' }),
        outer: await window.__TAURI_INTERNALS__.invoke('plugin:window|outer_size', { label: 'main' })
      }))()`);
      assert.deepEqual(bounds.inner, bounds.outer, 'Restore reintroduced a native inset frame.');
      return { maximized: true, restored: true, restoredShape, ...bounds };
    });
    await check('window-shape-survives-reload', async () => {
      const shape = await cdp.evaluate(`document.documentElement.dataset.windowShape`);
      const timeOrigin = await cdp.evaluate(`performance.timeOrigin`);
      await cdp.send('Page.reload');
      await waitFor(`performance.timeOrigin !== ${timeOrigin} && document.readyState === 'complete' && document.documentElement.dataset.windowShape === ${JSON.stringify(shape)} && document.querySelectorAll('.host-card').length === 4`, 'Page load did not republish the window contour.');
      return { shape };
    });
    await check('neutral-surface-and-accent', async () => {
      const styles = await cdp.evaluate(`({ background: getComputedStyle(document.querySelector('main.content')).backgroundColor,
        accent: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim(),
        primaryTransform: getComputedStyle(document.querySelector('.page-header .btn-primary')).transform })`);
      assert.equal(styles.background, 'rgb(243, 243, 243)');
      assert.equal(styles.accent.toLowerCase(), '#2c2c2c');
      assert.equal(styles.primaryTransform, 'none', 'Idle primary buttons must release their transform.');
      return styles;
    });
    await check('no-perpetual-decorative-animation', async () => {
      const state = await animations();
      assert.equal(state.infinite, 0, 'Found an animation with infinite iterations.');
      return state;
    });
    report.idleBefore = await heapMetrics();
    await capture('hosts.png');

    await check('primary-hover-press-and-host-editor', async () => {
      const position = await point('.page-header .btn-primary');
      await move(position);
      await delay(250);
      const hovered = await cdp.evaluate(`getComputedStyle(document.querySelector('.page-header .btn-primary')).transform`);
      assert(report.runtime.fineHover && !report.runtime.reducedMotion, 'Motion checks require a fine pointer and no-preference media emulation.');
      assert.notEqual(hovered, 'none', 'Primary button did not respond to hover.');
      await capture('hover.png');
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', buttons: 1, clickCount: 1, ...position });
      let pressed;
      try {
        await delay(110);
        pressed = await cdp.evaluate(`getComputedStyle(document.querySelector('.page-header .btn-primary')).transform`);
        assert.notEqual(pressed, 'none', 'Primary button did not respond to press.');
        assert.notEqual(pressed, hovered, 'Primary button did not respond to press.');
        await capture('pressed.png');
      } finally {
        await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', buttons: 0, clickCount: 1, ...position });
      }
      await waitFor(`!!document.querySelector('dialog.host-editor[open]')`, 'New-host editor did not open.');
      await delay(350);
      await capture('host-editor.png');
      assert(await cdp.evaluate(`document.querySelector('dialog.host-editor').contains(document.activeElement)`), 'Editor did not contain keyboard focus.');
      await click('dialog.host-editor .host-editor-fields input');
      await cdp.send('Input.insertText', { text: 'discard-only.invalid' });
      await key('Escape');
      await waitFor(`!!document.querySelector('dialog.host-dialog[open]')`, 'Dirty editor did not ask to discard.');
      await click('dialog.host-dialog .host-danger-button');
      await waitFor(`document.querySelectorAll('dialog[open]').length === 0`, 'Discard did not close the editor.');
      assert.equal(await cdp.evaluate(`document.querySelectorAll('.host-card').length`), 4, 'Discard changed the fixture host count.');
      return { hovered, pressed, dirtyDiscardConfirmed: true };
    });

    await check('floating-tooltip-and-escape', async () => {
      await move({ x: 4, y: 4 });
      await move(await point('.host-card .host-more'));
      await waitFor(`!!document.querySelector('#surface-hint[role="tooltip"]') && document.querySelector('.host-card .host-more').getAttribute('aria-describedby')?.split(' ').includes('surface-hint')`, 'Hover tooltip did not describe the host menu after its delay.');
      await delay(180);
      const tooltip = await cdp.evaluate(`(() => {
        const element = document.querySelector('#surface-hint');
        const rect = element.getBoundingClientRect();
        return { described: document.querySelector('.host-card .host-more').getAttribute('aria-describedby')?.split(' ').includes('surface-hint'),
          opacity: Number(getComputedStyle(element).opacity),
          withinViewport: rect.width > 0 && rect.height > 0 && rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight };
      })()`);
      assert(tooltip.described, 'Tooltip did not describe its control.');
      assert(tooltip.opacity >= 0.99, 'Tooltip entrance animation did not finish before capture.');
      assert(tooltip.withinViewport, 'Tooltip is outside the visible viewport.');
      await capture('tooltip.png');
      await key('Escape');
      await waitFor(`!document.querySelector('#surface-hint')`, 'Escape did not dismiss the tooltip.');
      assert(await cdp.evaluate(`!document.querySelector('.host-card .host-more').getAttribute('aria-describedby')?.split(' ').includes('surface-hint')`), 'Tooltip left a stale accessible description.');
      return { displayed: true, dismissed: true, descriptionCleaned: true, ...tooltip };
    });

    await check('group-filter-production-and-all', async () => {
      await click('.hosts-groups .pill .host-group-label', { text: 'Production' });
      await waitFor(`document.querySelectorAll('.host-card').length === 2`, 'Production did not show two fixture hosts.');
      await click('.hosts-groups .pill', { index: 0 });
      await waitFor(`document.querySelectorAll('.host-card').length === 4`, 'All did not restore all four fixture hosts.');
      return { production: 2, all: 4 };
    });

    await check('search-empty-state-and-clear', async () => {
      await click('.hosts-search input');
      await cdp.send('Input.insertText', { text: 'fixture-no-match-62843' });
      await waitFor(`document.querySelectorAll('.host-card').length === 0 && !!document.querySelector('.empty-state')`, 'Search did not show its empty state.');
      await click('.hosts-clear');
      await waitFor(`document.querySelectorAll('.host-card').length === 4`, 'Clear search did not restore all four fixture hosts.');
      return { emptyState: true, restored: 4 };
    });

    await check('host-context-menu-and-escape', async () => {
      await click('.host-card', { button: 'right' });
      await waitFor(`!!document.querySelector('.menu[role="menu"]')`, 'Host context menu did not open.');
      assert(await cdp.evaluate(`document.querySelector('.menu').contains(document.activeElement)`), 'Context menu did not receive focus.');
      await key('Escape');
      await waitFor(`!document.querySelector('.menu')`, 'Escape did not dismiss the context menu.');
      return { opened: true, focused: true, dismissed: true };
    });

    await check('keyboard-focus-visible', async () => {
      await click('.hosts-search input');
      await key('Tab');
      assert(await cdp.evaluate(`document.activeElement.matches('.hosts-view-toggle button:first-child:focus-visible')`), 'Grid view did not receive visible keyboard focus.');
      await key('Tab');
      assert(await cdp.evaluate(`document.activeElement.matches('.hosts-view-toggle button:last-child:focus-visible')`), 'List view did not receive visible keyboard focus.');
      await key('Tab');
      const focus = await cdp.evaluate(`(() => { const element = document.activeElement; const style = getComputedStyle(element);
        return { hostCard: element.matches('.host-card'), visible: element.matches(':focus-visible'),
          outlineStyle: style.outlineStyle, outlineWidth: style.outlineWidth }; })()`);
      assert(focus.hostCard && focus.visible, 'Tab did not put visible keyboard focus on a host card.');
      assert.notEqual(focus.outlineStyle, 'none');
      assert(parseFloat(focus.outlineWidth) > 0, 'Keyboard focus has no visible outline.');
      return focus;
    });

    await check('minimum-window-size-and-overflow', async () => {
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 900, height: 600, deviceScaleFactor: 1, mobile: false });
      await delay(350);
      const layout = await cdp.evaluate(`({ width: innerWidth, height: innerHeight,
        overflow: ['html', 'body', '#root', '.app', 'main.content', '.hosts-page'].map(selector => {
          const element = document.querySelector(selector);
          return { selector, clientWidth: element.clientWidth, scrollWidth: element.scrollWidth };
        }) })`);
      assert.equal(layout.width, 900);
      assert.equal(layout.height, 600);
      assert(layout.overflow.every(item => item.scrollWidth <= item.clientWidth + 1), 'Minimum window size has horizontal page overflow.');
      await capture('minimum-size.png');
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1240, height: 800, deviceScaleFactor: 1, mobile: false });
      await waitFor(`innerWidth === 1240 && innerHeight === 800`, 'Viewport did not restore.');
      return layout;
    });

    await check('keychain-page', () => navigate(1, 'main .feature-heading h1', 'keychain.png'));
    await check('snippets-page', () => navigate(2, 'main .feature-heading h1', 'snippets.png'));
    await check('settings-page', () => navigate(5, 'main .settings-heading h1', 'settings.png'));

    await check('reduced-motion-preference', async () => {
      await navigate(0, 'main .page-header h1');
      await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
      mediaOverridden = true;
      await move(await point('.page-header .btn-primary'));
      await delay(100);
      const state = await cdp.evaluate(`({ matches: matchMedia('(prefers-reduced-motion: reduce)').matches,
        hasFocus: document.hasFocus(), motionPaused: document.documentElement.dataset.motionPaused,
        primaryTransform: getComputedStyle(document.querySelector('.page-header .btn-primary')).transform,
        pageAnimation: getComputedStyle(document.querySelector('.hosts-page')).animationName,
        cardAnimation: getComputedStyle(document.querySelector('.host-card')).animationName,
        primaryTransition: getComputedStyle(document.querySelector('.page-header .btn-primary')).transitionDuration })`);
      assert(state.matches, 'Reduced-motion media preference was not emulated.');
      assert.equal(state.primaryTransform, 'none');
      assert.equal(state.pageAnimation, 'none');
      assert.equal(state.cardAnimation, 'none');
      assert.equal(state.primaryTransition, '0s');
      return state;
    });
    await cdp.send('Emulation.setEmulatedMedia', { features: [] });
    mediaOverridden = false;
    await move({ x: 4, y: 4 });
    await delay(800);
    report.idleAfter = await heapMetrics();
    await check('final-idle-and-fixture-preserved', async () => {
      const state = await animations();
      assert.equal(state.infinite, 0);
      assert.equal(state.running, 0, 'Decorative work continued after the idle settling period.');
      assert.equal(cdp.exceptions.length, 0, 'The WebView reported a JavaScript runtime exception.');
      const preserved = await cdp.evaluate(`JSON.stringify([...document.querySelectorAll('.host-card h3')].map(element => element.textContent.trim()).sort()) === ${JSON.stringify(JSON.stringify(fixtureNames))}`);
      assert(preserved, 'Fixture hosts changed during UI-only checks.');
      return { ...state, fixturePreserved: true, runtimeExceptions: cdp.exceptions.length };
    });
  } catch (error) {
    report.error = error.message;
    if (fixtureVerified && cdp) {
      try { await capture('failure.png'); } catch { /* Preserve the original failure. */ }
    }
  } finally {
    if (cdp) {
      if (mediaOverridden) { try { await cdp.send('Emulation.setEmulatedMedia', { features: [] }); } catch { /* Connection may be gone. */ } }
      if (metricsOverridden) { try { await cdp.send('Emulation.clearDeviceMetricsOverride'); } catch { /* Connection may be gone. */ } }
      if (focusOverridden) { try { await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: false }); } catch { /* Connection may be gone. */ } }
      if (fixtureVerified) { try { await cdp.send('Emulation.setDefaultBackgroundColorOverride'); } catch { /* Connection may be gone. */ } }
      cdp.close();
    }
    report.durationMs = Math.round(performance.now() - started);
    report.passed = !report.error && report.checks.every(item => item.status !== 'failed');
    await writeFile(resolve(outputDir, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  }
  return report;
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  try {
    const { values } = parseArgs({ options: {
      port: { type: 'string', default: '9337' }, output: { type: 'string' }, help: { type: 'boolean' },
    } });
    if (values.help) console.log('node native/scripts/verify-ui.mjs --port=9337 --output=<ignored E: test-output directory>');
    else {
      const report = await verifyUi({ port: Number(values.port), output: values.output });
      console.log(`${report.passed ? 'PASS' : 'FAIL'} ${report.checks.filter(item => item.status === 'passed').length}/${report.checks.length} checks; report.json; ${report.screenshots.length} screenshots.`);
      if (!report.passed) process.exitCode = 1;
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

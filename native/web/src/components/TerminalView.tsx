import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { sessionInput, sessionResize, subscribeSession } from "../api/sessions";
import { t } from "../i18n";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

const FONT_FAMILY = '"Cascadia Mono", Consolas, monospace';
// xterm's default separators plus full-width CJK punctuation, so double-click selects "地址" without "：".
const WORD_SEPARATOR = " ()[]{}',\"`：，。；！？、（）【】《》「」“”‘’";

function terminalTheme() {
  const style = getComputedStyle(document.documentElement);
  const color = (name: string) => style.getPropertyValue(`--term-${name}`).trim();
  return {
    background: color("background"),
    foreground: color("foreground"),
    cursor: color("cursor"),
    cursorAccent: color("cursor-accent"),
    selectionBackground: color("selection-background"),
    selectionForeground: color("selection-foreground"),
    black: color("black"),
    red: color("red"),
    green: color("green"),
    yellow: color("yellow"),
    blue: color("blue"),
    magenta: color("magenta"),
    cyan: color("cyan"),
    white: color("white"),
    brightBlack: color("bright-black"),
    brightRed: color("bright-red"),
    brightGreen: color("bright-green"),
    brightYellow: color("bright-yellow"),
    brightBlue: color("bright-blue"),
    brightMagenta: color("bright-magenta"),
    brightCyan: color("bright-cyan"),
    brightWhite: color("bright-white"),
  };
}

export function TerminalView({ sessionId, visible, ended, local, blocked, onReconnect }: {
  sessionId: string;
  visible: boolean;
  ended: boolean;
  local: boolean;
  blocked: boolean;
  onReconnect: () => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | null>(null);
  const fitVisible = useRef<() => void>(() => {});
  const current = useRef({ visible, ended, local, blocked, onReconnect });
  current.current = { visible, ended, local, blocked, onReconnect };
  const [error, setError] = useState("");
  const [paste, setPaste] = useState<string | null>(null);

  useEffect(() => {
    const element = container.current!;
    const xterm = new Terminal({
      fontFamily: FONT_FAMILY, fontSize: 14, cursorBlink: true,
      scrollback: 10000, allowProposedApi: false, wordSeparator: WORD_SEPARATOR,
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.loadAddon(new WebLinksAddon());
    xterm.open(element);
    terminal.current = xterm;
    const themeObserver = new MutationObserver(() => { xterm.options.theme = terminalTheme(); });
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    let disposed = false;
    let frame = 0;
    const report = (reason: unknown) => { if (!disposed) setError(String(reason)); };
    function send(data: string) {
      if (!current.current.ended) void sessionInput(sessionId, data).catch(report);
    }
    function syncSize() {
      if (current.current.visible && !current.current.ended && element.clientWidth && element.clientHeight) {
        void sessionResize(sessionId, xterm.cols, xterm.rows).catch(report);
      }
    }
    fitVisible.current = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (disposed || !current.current.visible || !element.clientWidth || !element.clientHeight) return;
        const atBottom = xterm.buffer.active.viewportY === xterm.buffer.active.baseY;
        // A same-size resize remeasures characters if open() ran under display:none.
        xterm.resize(xterm.cols, xterm.rows);
        const dimensions = fit.proposeDimensions();
        // FitAddon clamps tiny containers to 2x1; do not reflow or send that geometry.
        if (!dimensions || !Number.isFinite(dimensions.cols) || !Number.isFinite(dimensions.rows)
          || dimensions.cols <= 2 || dimensions.rows <= 1) return;
        fit.fit();
        syncSize();
        if (atBottom) xterm.scrollToBottom();
        // fit() does not repaint when the geometry is unchanged after being hidden.
        xterm.refresh(0, xterm.rows - 1);
        if (!current.current.blocked) xterm.focus();
      });
    };
    function requestPaste(text: string) {
      if (disposed || current.current.ended || current.current.blocked) return;
      if (new TextEncoder().encode(text).length > 1024 * 1024) { setError(t("Pasted text must not exceed 1 MB")); return; }
      const lines = text.replace(/\r\n?/g, "\n").split("\n").length;
      if (lines > 1) setPaste(text);
      else xterm.paste(text.replace(/\x1b/g, ""));
    }
    async function readClipboard() {
      try { requestPaste(await navigator.clipboard.readText()); } catch { report(t("Cannot access clipboard")); }
    }
    async function copySelection() {
      if (!xterm.hasSelection()) return;
      try {
        await navigator.clipboard.writeText(xterm.getSelection());
        if (!disposed) xterm.clearSelection();
      } catch { report(t("Cannot access clipboard")); }
    }
    xterm.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const copy = !event.altKey && event.ctrlKey && ((key === "c" && (event.shiftKey || xterm.hasSelection())) || (!event.shiftKey && key === "insert"));
      const pasteShortcut = !event.altKey && ((event.ctrlKey && key === "v") || (!event.ctrlKey && event.shiftKey && key === "insert"));
      if (copy || pasteShortcut) {
        event.stopPropagation();
        // Paste shortcuts keep their default action: the native paste event below carries the text without
        // a clipboard read permission, which WebView2 may otherwise deny on first use.
        if (copy) { event.preventDefault(); if (event.type === "keydown") void copySelection(); }
        return false;
      }
      if (current.current.ended) {
        if (event.type === "keydown" && event.key === "Enter" && !event.ctrlKey && !event.altKey && !event.shiftKey && !current.current.local) {
          event.preventDefault();
          current.current.onReconnect();
        }
        return false;
      }
      return !current.current.blocked;
    });
    // Capture native paste as well, so browser paste cannot bypass confirmation.
    function nativePaste(event: ClipboardEvent) {
      event.preventDefault();
      event.stopImmediatePropagation();
      requestPaste(event.clipboardData?.getData("text/plain") ?? "");
    }
    function contextMenu(event: MouseEvent) {
      event.preventDefault();
      event.stopPropagation();
      if (xterm.hasSelection()) void copySelection(); else void readClipboard();
    }
    function rightMouseDown(event: MouseEvent) {
      if (event.button === 2) { event.preventDefault(); event.stopImmediatePropagation(); }
    }
    element.addEventListener("paste", nativePaste, true);
    element.addEventListener("contextmenu", contextMenu, true);
    element.addEventListener("mousedown", rightMouseDown, true);
    const input = xterm.onData(send);
    const binary = xterm.onBinary(send);
    const unsubscribe = subscribeSession(sessionId, (event) => { if (event.type === "output") xterm.write(event.data); });
    const observer = new ResizeObserver(() => fitVisible.current());
    observer.observe(element);
    fitVisible.current();
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      themeObserver.disconnect();
      unsubscribe();
      input.dispose();
      binary.dispose();
      element.removeEventListener("paste", nativePaste, true);
      element.removeEventListener("contextmenu", contextMenu, true);
      element.removeEventListener("mousedown", rightMouseDown, true);
      terminal.current = null;
      fitVisible.current = () => {};
      xterm.dispose();
    };
  }, [sessionId]);

  useEffect(() => {
    if (visible) fitVisible.current();
    else setPaste(null);
  }, [visible, ended, blocked]);

  return (
    <div className="terminal-view" hidden={!visible}>
      <div ref={container} className="xterm-container" aria-label={t("Terminal")} />
      {error && <div className="terminal-notice terminal-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span><button className="icon-button icon-button-sm" onClick={() => setError("")} aria-label={t("Dismiss")}><Icon name="close" size={14} /></button></div>}
      {ended && <button className="terminal-notice reconnect-hint" disabled={local} onClick={onReconnect}>{t(local ? "Local terminal exited" : "Press Enter to reconnect")}</button>}
      {paste !== null && <Dialog title={t("Paste multiple lines")} onClose={() => { setPaste(null); terminal.current?.focus(); }}>
        <p>{t("The clipboard contains {lines} lines. Paste them into the terminal?", { lines: paste.replace(/\r\n?/g, "\n").split("\n").length })}</p>
        <div className="dialog-actions">
          <button autoFocus className="btn" onClick={() => { setPaste(null); terminal.current?.focus(); }}>{t("Cancel")}</button>
          <button className="btn btn-primary" onClick={() => {
            if (!ended && !blocked) terminal.current?.paste(paste.replace(/\x1b/g, ""));
            setPaste(null);
            terminal.current?.focus();
          }}>{t("Paste")}</button>
        </div>
      </Dialog>}
    </div>
  );
}

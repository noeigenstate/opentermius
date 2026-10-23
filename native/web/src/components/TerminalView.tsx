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
      scrollback: 10000, allowProposedApi: false,
      theme: {
        background: "#ffffff", foreground: "#1d1d1f", cursor: "#007aff", cursorAccent: "#ffffff",
        selectionBackground: "#cce4ff", selectionForeground: "#1d1d1f",
        black: "#263238", red: "#b91c1c", green: "#166534", yellow: "#854d0e",
        blue: "#1d4ed8", magenta: "#86198f", cyan: "#0e7490", white: "#cbd5e1",
        brightBlack: "#64748b", brightRed: "#dc2626", brightGreen: "#15803d", brightYellow: "#a16207",
        brightBlue: "#2563eb", brightMagenta: "#a21caf", brightCyan: "#0891b2", brightWhite: "#f1f5f9",
      },
    });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.loadAddon(new WebLinksAddon());
    xterm.open(element);
    terminal.current = xterm;
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
        fit.fit();
        syncSize();
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
        event.preventDefault();
        event.stopPropagation();
        if (event.type === "keydown") { if (copy) void copySelection(); else void readClipboard(); }
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
          <button autoFocus onClick={() => { setPaste(null); terminal.current?.focus(); }}>{t("Cancel")}</button>
          <button className="primary-button" onClick={() => {
            if (!ended && !blocked) terminal.current?.paste(paste.replace(/\x1b/g, ""));
            setPaste(null);
            terminal.current?.focus();
          }}>{t("Paste")}</button>
        </div>
      </Dialog>}
    </div>
  );
}

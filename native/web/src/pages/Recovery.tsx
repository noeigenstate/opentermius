import { useEffect, useRef, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { recoveryGenerate, recoveryRestore, type RestoreMode } from "../api/data";
import { t } from "../i18n";
import { Icon } from "../icons";

export function Recovery({ onClose, onRestored }: { onClose: () => void; onRestored: () => Promise<void> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [tab, setTab] = useState<"generate" | "restore">("generate");
  const [passphrase, setPassphrase] = useState("");
  const [includeKeys, setIncludeKeys] = useState(true);
  const [code, setCode] = useState("");
  const [pasted, setPasted] = useState("");
  const [mode, setMode] = useState<RestoreMode>("merge");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  useEffect(() => { dialog.current?.showModal(); }, []);

  async function generate() {
    setBusy(true); setError(""); setMessage(""); setCode("");
    const password = passphrase;
    setPassphrase("");
    try {
      setCode(await recoveryGenerate(password, includeKeys));
      setMessage(t("Recovery code generated. Copy it to a safe location."));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function restore() {
    setBusy(true); setError(""); setMessage("");
    try {
      if (mode === "replace" && !await ask(t("Replace hosts, keys, snippets and preferences with this backup? Conflicting local fingerprints are kept."), {
        title: t("Replace local data"), kind: "warning", okLabel: t("Replace and restore"), cancelLabel: t("Cancel"),
      })) return;
      const password = passphrase;
      setPassphrase("");
      const summary = await recoveryRestore(pasted, password, mode);
      setPasted("");
      setMessage(summary);
      await onRestored();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function copy() {
    setError("");
    try {
      await navigator.clipboard.writeText(code);
      setMessage(t("Recovery code copied. The clipboard may contain credentials; clear it soon."));
    } catch { setError(t("Cannot copy. Select the recovery code and copy it manually.")); }
  }

  return <dialog ref={dialog} className="dialog dialog-wide" aria-labelledby="recovery-title" onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="dialog-head"><h2 id="recovery-title">{t("Backup & Restore")}</h2><button className="icon-button icon-button-lg" aria-label={t("Close")} disabled={busy} onClick={onClose}><Icon name="close" size={20} /></button></div>
    <fieldset disabled={busy} className="settings-fields">
      <div className="segmented" role="group" aria-label={t("Backup & Restore")}>
        <button aria-pressed={tab === "generate"} onClick={() => { setTab("generate"); setPassphrase(""); setError(""); setMessage(""); }}>{t("Generate backup")}</button>
        <button aria-pressed={tab === "restore"} onClick={() => { setTab("restore"); setPassphrase(""); setCode(""); setError(""); setMessage(""); }}>{t("Restore backup")}</button>
      </div>
      <p className="settings-hint">{t("Keep recovery codes private. Without a passphrase, anyone with the code can restore its contents.")}</p>
      <label className="settings-row"><span>{t("Passphrase (optional)")}</span><input type="password" autoComplete="off" value={passphrase} onChange={e => setPassphrase(e.target.value)} /></label>
      {tab === "generate" ? <>
        <label className="settings-row"><span>{t("Include saved credentials and private keys")}</span><input type="checkbox" role="switch" checked={includeKeys} onChange={e => setIncludeKeys(e.target.checked)} /></label>
        <div className="settings-actions"><button className="btn btn-primary" onClick={() => void generate()}>{t("Generate code")}</button><button className="btn" disabled={!code} onClick={() => void copy()}><Icon name="copy" size={14} />{t("Copy code")}</button></div>
        <textarea className="settings-code" aria-label={t("Generated recovery code")} readOnly value={code} spellCheck={false} />
      </> : <>
        <label className="settings-code-label">{t("Paste recovery code")}<textarea className="settings-code" value={pasted} onChange={e => setPasted(e.target.value)} spellCheck={false} /></label>
        <label className="settings-row"><span>{t("Restore mode")}</span><select aria-label={t("Restore mode")} value={mode} onChange={e => setMode(e.target.value as RestoreMode)}><option value="merge">{t("Merge (keep existing items)")}</option><option value="replace">{t("Replace local data")}</option></select></label>
        <button className="btn btn-primary" disabled={!pasted.trim()} onClick={() => void restore()}>{t("Restore")}</button>
      </>}
    </fieldset>
    {busy && <p className="feature-loading" role="status">{t("Working…")}</p>}
    {message && <div className="banner banner-ok" role="status"><Icon name="check" size={16} /><span>{message}</span></div>}
    {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
  </dialog>;
}

import { useEffect, useState, type FormEvent } from "react";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import * as api from "../api/keychain";
import { t } from "../i18n";
import { Icon } from "../icons";

export function KeychainPage() {
  const [keys, setKeys] = useState<api.KeyInfo[]>([]);
  const [search, setSearch] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<"generate" | "import" | "rename" | null>(null);
  const [selected, setSelected] = useState<api.KeyInfo | null>(null);
  const [label, setLabel] = useState("");
  const [kind, setKind] = useState<api.KeyKind>("ed25519");
  const [path, setPath] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [requiresPassphrase, setRequiresPassphrase] = useState(false);
  const refresh = () => api.listKeys().then(setKeys);
  useEffect(() => { refresh().catch(e => setError(String(e))).finally(() => setLoading(false)); }, []);

  async function act(action: () => Promise<unknown>, success = "") {
    setBusy(true); setError(""); setMessage("");
    try { await action(); if (success) setMessage(success); } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  function closeEditor() { setEditor(null); setSelected(null); setPassphrase(""); }
  function newKey() { setSelected(null); setLabel(""); setKind("ed25519"); setError(""); setEditor("generate"); }
  async function chooseImport() {
    await act(async () => {
      const chosen = await open({ multiple: false, directory: false, title: t("Import private key") });
      if (!chosen) return;
      setRequiresPassphrase(await api.keyRequiresPassphrase(chosen));
      setSelected(null); setPath(chosen); setLabel(""); setPassphrase(""); setEditor("import");
    });
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    await act(async () => {
      if (editor === "generate") await api.generateKey(label, kind);
      else if (editor === "import") {
        const password = passphrase; setPassphrase("");
        await api.importKey(path, label, password);
      } else if (selected) await api.renameKey(selected.id, label);
      closeEditor(); await refresh();
    }, t("Key saved"));
  }

  async function exportKey(key: api.KeyInfo) {
    await act(async () => {
      const destination = await save({ title: t("Export public key"), defaultPath: "ssh-key.pub" });
      if (!destination) return;
      await api.exportPublicKey(key.id, destination);
      setMessage(t("Public key exported"));
    });
  }

  async function removeKey(key: api.KeyInfo) {
    await act(async () => {
      const accepted = await confirm(t("Delete key {name}?", { name: key.label }), {
        title: t("Delete key"), kind: "warning",
      });
      if (!accepted) return;
      await api.deleteKey(key.id);
      if (selected?.id === key.id) closeEditor();
      await refresh();
    });
  }
  const query = search.trim().toLowerCase();
  const visible = keys.filter(key => [key.label, key.kind, key.fingerprint, ...key.usedBy].some(value => value.toLowerCase().includes(query)));
  return <section className="page feature-page">
    <header className="feature-header glass">
      <h1>{t("Keychain")}</h1>
      <label className="search glass"><Icon name="search" /><input aria-label={t("Search keys")} placeholder={t("Search keys")} value={search} onChange={e => setSearch(e.target.value)} /></label>
      <button className="feature-button" disabled={busy} onClick={() => void chooseImport()}>{t("Import key")}</button>
      <button className="feature-button primary" disabled={busy} onClick={newKey}><Icon name="plus" />{t("New key")}</button>
    </header>
    {error && <div className="feature-error" role="alert">{error}</div>}
    {message && <p className="feature-notice" role="status">{message}</p>}
    {busy && <p role="status">{t("Working…")}</p>}
    {editor && <form className="feature-editor" onSubmit={event => void submit(event)} aria-label={t(editor === "rename" ? "Rename key" : editor === "import" ? "Import private key" : "New key")}>
      <h2>{t(editor === "rename" ? "Rename key" : editor === "import" ? "Import private key" : "New key")}</h2>
      <label>{t("Label")}<input autoFocus value={label} onChange={e => setLabel(e.target.value)} maxLength={128} required={editor === "rename"} disabled={busy} /></label>
      {editor === "generate" && <label>{t("Key type")}<select value={kind} onChange={e => setKind(e.target.value as api.KeyKind)} disabled={busy}><option value="ed25519">Ed25519</option><option value="rsa3072">RSA 3072</option><option value="rsa4096">RSA 4096</option></select></label>}
      {editor === "import" && <label>{requiresPassphrase ? t("Passphrase") : t("Passphrase (optional)")}<input type="password" autoComplete="off" required={requiresPassphrase} value={passphrase} onChange={e => setPassphrase(e.target.value)} disabled={busy} /></label>}
      <div className="feature-actions"><button className="feature-button" type="button" disabled={busy} onClick={closeEditor}>{t("Cancel")}</button><button className="feature-button primary" disabled={busy}>{t(editor === "generate" ? "Generate" : "Save")}</button></div>
    </form>}
    {loading ? <p role="status">{t("Loading…")}</p> : visible.length === 0 ? <div className="feature-empty"><Icon name="keychain" size={40} /><h2>{t(keys.length ? "No matching keys" : "No keys yet")}</h2><p>{t("Generate a key or import an existing private key.")}</p><button className="feature-button primary" disabled={busy} onClick={newKey}>{t("New key")}</button></div> : <div className="feature-list">
      {visible.map(key => <article className="feature-row" key={key.id}>
        <div className="feature-row-title"><Icon name="keychain" size={22} /><h2>{key.label}</h2><span className="feature-tag">{key.kind}</span></div>
        <p className="feature-mono feature-wrap">{key.fingerprint}</p>
        <p className="feature-secondary">{t("Used by")}: {key.usedBy.join(", ") || t("Not in saved hosts")}</p>
        <details><summary>{t("Public key")}</summary><pre className="feature-wrap">{key.publicKey}</pre></details>
        <div className="feature-actions">
          <button className="feature-button" disabled={busy} onClick={() => void act(() => navigator.clipboard.writeText(key.publicKey), t("Public key copied"))}>{t("Copy public key")}</button>
          <button className="feature-button" disabled={busy} onClick={() => void exportKey(key)}>{t("Export public key")}</button>
          <button className="feature-button" disabled={busy} onClick={() => { setSelected(key); setLabel(key.label); setEditor("rename"); }}>{t("Rename")}</button>
          <button className="feature-button danger" disabled={busy} onClick={() => void removeKey(key)}>{t("Delete")}</button>
        </div>
      </article>)}
    </div>}
  </section>;
}

import { useEffect, useState, type FormEvent } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import * as api from "../api/snippets";
import { t } from "../i18n";
import { Icon } from "../icons";

export function SnippetsPage({ onRun }: { onRun?: (script: string) => void | Promise<void> }) {
  const [snippets, setSnippets] = useState<api.Snippet[]>([]);
  const [search, setSearch] = useState("");
  const [draft, setDraft] = useState<api.SnippetDraft | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const refresh = () => api.listSnippets().then(setSnippets);
  useEffect(() => { refresh().catch(e => setError(String(e))).finally(() => setLoading(false)); }, []);
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await action(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  function create() { setDraft({ id: "", name: "", script: "", tags: "" }); setError(""); }
  async function submit(event: FormEvent) {
    event.preventDefault(); if (!draft) return;
    await act(async () => { await api.saveSnippet(draft); setDraft(null); await refresh(); });
  }

  async function removeSnippet(snippet: api.Snippet) {
    await act(async () => {
      const accepted = await confirm(t("Delete snippet {name}?", { name: snippet.name }), {
        title: t("Delete snippet"), kind: "warning",
      });
      if (!accepted) return;
      await api.deleteSnippet(snippet.id);
      if (draft?.id === snippet.id) setDraft(null);
      await refresh();
    });
  }
  const query = search.trim().toLowerCase();
  const visible = snippets.filter(snippet => [snippet.name, snippet.script, ...snippet.tags].some(value => value.toLowerCase().includes(query)));
  return <section className="page feature-page">
    <header className="page-header">
      <div><h1>{t("Snippets")}</h1><p>{t("Keep your frequently used commands within reach.")}</p></div>
      <div className="page-actions"><button className="btn btn-l btn-primary" disabled={busy} onClick={create}><Icon name="plus" size={16} />{t("New snippet")}</button></div>
    </header>
    <div className="page-controls"><label className="search"><Icon name="search" size={18} /><input aria-label={t("Search snippets")} placeholder={t("Search snippets")} value={search} onChange={e => setSearch(e.target.value)} /></label></div>
    {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
    {draft && <form className="feature-editor" aria-label={t(draft.id ? "Edit snippet" : "New snippet")} onSubmit={event => void submit(event)}>
      <h2>{t(draft.id ? "Edit snippet" : "New snippet")}</h2>
      <label className="field">{t("Name")}<input autoFocus required maxLength={128} value={draft.name} disabled={busy} onChange={e => setDraft({ ...draft, name: e.target.value })} /></label>
      <label className="field">{t("Script")}<textarea required rows={8} className="feature-mono" value={draft.script} disabled={busy} spellCheck={false} onChange={e => setDraft({ ...draft, script: e.target.value })} /></label>
      <label className="field">{t("Tags")}<input value={draft.tags} disabled={busy} placeholder={t("Separate tags with commas")} onChange={e => setDraft({ ...draft, tags: e.target.value })} /></label>
      <div className="feature-actions"><button className="btn" type="button" disabled={busy} onClick={() => setDraft(null)}>{t("Cancel")}</button><button className="btn btn-primary" disabled={busy}>{t("Save")}</button></div>
    </form>}
    {loading ? <p className="feature-loading" role="status">{t("Loading…")}</p> : visible.length === 0 ? <div className="empty-state"><span className="empty-glyph"><Icon name="snippets" size={32} /></span><h2>{t(snippets.length ? "No matching snippets" : "No snippets yet")}</h2><p>{t("Save commands you run often and paste them into any session.")}</p><button className="btn btn-l btn-primary" disabled={busy} onClick={create}><Icon name="plus" size={16} />{t("New snippet")}</button></div> : <div className="card-grid">
      {visible.map(snippet => <article className="feature-card" key={snippet.id}>
        <div className="feature-card-head"><span className="feature-tile"><Icon name="snippets" size={20} /></span><h2>{snippet.name}</h2>{snippet.tags.map(tag => <span className="chip" key={tag}>{tag}</span>)}</div>
        <pre className="feature-script">{snippet.script}</pre>
        <footer className="feature-card-foot">
          <button className="btn btn-s btn-primary" disabled={busy || !onRun} title={!onRun ? t("Open a terminal to paste a snippet") : undefined} onClick={() => void act(async () => { await onRun?.(snippet.script); })}><Icon name="prompt" size={14} />{t("Paste into terminal")}</button>
          <button className="btn btn-s" disabled={busy} onClick={() => setDraft({ ...snippet, tags: snippet.tags.join(", ") })}><Icon name="edit" size={14} />{t("Edit")}</button>
          <button className="btn btn-s btn-danger feature-push" disabled={busy} onClick={() => void removeSnippet(snippet)}><Icon name="trash" size={14} />{t("Delete")}</button>
        </footer>
      </article>)}
    </div>}
  </section>;
}

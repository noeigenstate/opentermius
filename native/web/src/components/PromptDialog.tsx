import { useState } from "react";
import { answerPrompt, type SessionPrompt } from "../api/sessions";
import { t } from "../i18n";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

/** Prompt text that only repeats the dialog title ("Password", "Password:") adds nothing. */
const repeats = (text: string, title: string) => text.trim().replace(/[:：]$/, "").toLowerCase() === title.trim().toLowerCase();

export function PromptDialog({ prompt, onAnswered }: { prompt: SessionPrompt; onAnswered: (promptId: string) => void }) {
  const fields = prompt.kind === "hostKey" ? [] : prompt.kind === "keyboardInteractive" ? prompt.prompts ?? [] : [{
    prompt: t(prompt.kind === "username" ? "Username" : prompt.kind === "passphrase" ? "Passphrase" : "Password"),
    echo: prompt.kind === "username",
  }];
  const [answers, setAnswers] = useState<string[]>(() => fields.map(() => ""));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function answer(accepted: boolean) {
    if (busy) return;
    setBusy(true);
    setError("");
    const submitted = accepted ? answers : [];
    setAnswers(fields.map(() => ""));
    try {
      await answerPrompt(prompt.promptId, accepted, submitted);
      onAnswered(prompt.promptId);
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    } finally {
      submitted.fill("");
    }
  }
  return (
    <Dialog title={prompt.title} busy={busy} onClose={() => { void answer(false); }}>
      <form onSubmit={(event) => { event.preventDefault(); void answer(true); }}>
        {prompt.message && !repeats(prompt.message, prompt.title) && <p className="prompt-message">{prompt.message}</p>}
        {prompt.kind === "hostKey" && <>
          <code className="dialog-code">{prompt.fingerprint}</code>
          <p>{t("Verify this fingerprint through a trusted channel before trusting it.")}</p>
        </>}
        {fields.map((field, index) => (
          <label className="field" key={index}>
            {!(fields.length === 1 && repeats(field.prompt, prompt.title)) && <span>{field.prompt}</span>}
            <input aria-label={field.prompt} autoFocus={index === 0} autoComplete="off" spellCheck={false} type={field.echo ? "text" : "password"}
              disabled={busy} required={prompt.kind === "username"} maxLength={8192} value={answers[index] ?? ""}
              onChange={(event) => setAnswers((current) => current.map((value, i) => i === index ? event.target.value : value))} />
          </label>
        ))}
        {error && <div className="banner banner-error" role="alert"><Icon name="alert" size={16} /><span>{error}</span></div>}
        <div className="dialog-actions">
          <button type="button" className="btn" disabled={busy} onClick={() => { void answer(false); }}>{t("Cancel")}</button>
          <button type="submit" className="btn btn-primary" disabled={busy}>{t(prompt.kind === "hostKey" ? "Trust and connect" : "Connect")}</button>
        </div>
      </form>
    </Dialog>
  );
}

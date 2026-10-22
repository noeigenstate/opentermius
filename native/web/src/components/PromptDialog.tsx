import { useState } from "react";
import { answerPrompt, type SessionPrompt } from "../api/sessions";
import { t } from "../i18n";
import { Dialog } from "./Dialog";

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
        <p className="prompt-message">{prompt.message}</p>
        {prompt.kind === "hostKey" && <>
          <code className="host-fingerprint">{prompt.fingerprint}</code>
          <p>{t("Verify this fingerprint through a trusted channel before trusting it.")}</p>
        </>}
        {fields.map((field, index) => (
          <label className="prompt-field" key={index}>
            <span>{field.prompt}</span>
            <input autoFocus={index === 0} autoComplete="off" spellCheck={false} type={field.echo ? "text" : "password"}
              disabled={busy} required={prompt.kind === "username"} maxLength={8192} value={answers[index] ?? ""}
              onChange={(event) => setAnswers((current) => current.map((value, i) => i === index ? event.target.value : value))} />
          </label>
        ))}
        {error && <p className="terminal-error" role="alert">{error}</p>}
        <div className="dialog-actions">
          <button type="button" disabled={busy} onClick={() => { void answer(false); }}>{t("Cancel")}</button>
          <button type="submit" className="primary-button" disabled={busy}>{t(prompt.kind === "hostKey" ? "Trust and connect" : "Connect")}</button>
        </div>
      </form>
    </Dialog>
  );
}

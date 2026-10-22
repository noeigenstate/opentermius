import { useEffect, useId, useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listKeychainKeys, saveHost, type HostDraft, type KeychainKey } from "../api/hosts";
import { t } from "../i18n";
import { HostModal } from "./Menu";

interface Props {
  initial: HostDraft;
  groups: string[];
  onClose: () => void;
  onSaved: (hostId: string) => void | Promise<void>;
  onConnect?: (hostId: string) => void;
}

export function HostEditor({ initial, groups, onClose, onSaved, onConnect }: Props) {
  const [draft, setDraft] = useState<HostDraft>({ ...initial, password: "" });
  const [keys, setKeys] = useState<KeychainKey[]>([]);
  const initialKeySource = initial.keyId || !initial.privateKeyPath ? "keychain" : "file";
  const [keySource, setKeySource] = useState(initialKeySource);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [discard, setDiscard] = useState(false);
  const id = useId();
  const dirty = keySource !== initialKeySource || JSON.stringify(draft) !== JSON.stringify({ ...initial, password: "" });

  useEffect(() => {
    let active = true;
    listKeychainKeys().then((value) => { if (active) setKeys(value); }).catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    if (!dirty) return;
    const guard = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ""; };
    window.addEventListener("beforeunload", guard);
    return () => window.removeEventListener("beforeunload", guard);
  }, [dirty]);

  const update = (fields: Partial<HostDraft>) => setDraft((value) => ({ ...value, ...fields }));
  const close = () => { if (!busy) { if (dirty) setDiscard(true); else onClose(); } };
  const browse = async () => {
    try {
      const path = await open({ title: t("Select SSH private key"), directory: false, multiple: false });
      if (typeof path === "string") update({ privateKeyPath: path, keyId: "" });
    } catch (reason) { setError(String(reason)); }
  };
  const save = async (connect: boolean) => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      const hostId = await saveHost({ ...draft, keyId: draft.authType === "key" && keySource === "keychain" ? draft.keyId : "", privateKeyPath: draft.authType === "key" && keySource === "file" ? draft.privateKeyPath : "" });
      // Drop the typed secret as soon as save succeeds, including when refresh fails.
      setDraft((value) => ({ ...value, password: "" }));
      await onSaved(hostId);
      onClose();
      if (connect) onConnect?.(hostId);
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const button = (event.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null;
    void save(button?.value === "connect");
  };

  return <>
    <HostModal titleId={`${id}-title`} panel onClose={close}>
      <form className="host-editor-form" onSubmit={submit}>
        <header className="host-editor-header">
          <div><p>{t("Hosts")}</p><h2 id={`${id}-title`}>{initial.id ? t("Edit host") : t("New host")}</h2></div>
          <button type="button" className="icon-button" disabled={busy} aria-label={t("Close")} onClick={close}>×</button>
        </header>
        <fieldset disabled={busy} className="host-editor-fields">
          <label className="host-field">{t("Address")}<input autoFocus value={draft.host} onChange={(event) => update({ host: event.target.value })} required autoComplete="off" placeholder={t("Host address")} /></label>
          <section className="host-editor-section">
            <h3>{t("General")}</h3>
            <label className="host-field">{t("Label")}<input value={draft.name} onChange={(event) => update({ name: event.target.value })} placeholder={t("Defaults to address")} /></label>
            <label className="host-field">{t("Parent group")}<input value={draft.group} onChange={(event) => update({ group: event.target.value })} list={`${id}-groups`} placeholder={t("No group")} /><datalist id={`${id}-groups`}>{groups.map((group) => <option key={group} value={group} />)}</datalist></label>
            <label className="host-field">{t("Tags")}<input value={draft.tags} onChange={(event) => update({ tags: event.target.value })} placeholder={t("Comma-separated tags")} /></label>
          </section>
          <section className="host-editor-section">
            <label className="host-port">{t("SSH port")}<input aria-label={t("SSH port")} type="number" min="1" max="65535" value={draft.port} placeholder="22" onChange={(event) => update({ port: event.target.value })} /></label>
          </section>
          <section className="host-editor-section">
            <h3>{t("Credentials")}</h3>
            <label className="host-field">{t("Username")}<input value={draft.username} onChange={(event) => update({ username: event.target.value })} autoComplete="off" /></label>
            <div className="host-segmented" role="group" aria-label={t("Authentication method")}>
              {(["password", "key"] as const).map((method) => <button type="button" key={method} aria-pressed={draft.authType === method} onClick={() => {
                if (draft.authType !== method) update({ authType: method, password: "" });
              }}>{t(method === "password" ? "Password" : "Key")}</button>)}
            </div>
            {draft.authType === "key" && <>
              <label className="host-field">{t("Key source")}<select aria-label={t("Key source")} value={keySource} onChange={(event) => { setKeySource(event.target.value); update({ password: "" }); }}><option value="keychain">{t("Keychain key")}</option><option value="file">{t("Private key file")}</option></select></label>
              {keySource === "keychain" ? <label className="host-field">{t("Keychain key")}<select aria-label={t("Keychain key")} value={draft.keyId} onChange={(event) => update({ keyId: event.target.value })} required>
                <option value="">{keys.length ? t("Select a key") : t("No Keychain keys")}</option>
                {draft.keyId && !keys.some((key) => key.id === draft.keyId) && <option value={draft.keyId}>{t("Unavailable key")}</option>}
                {keys.map((key) => <option key={key.id} value={key.id}>{key.label} · {key.kind}</option>)}
              </select></label> : <div className="host-file-row"><label className="host-field">{t("Private key file")}<input value={draft.privateKeyPath} onChange={(event) => update({ privateKeyPath: event.target.value })} required /></label><button type="button" className="host-button" onClick={() => void browse()}>{t("Browse")}</button></div>}
            </>}
            {(draft.authType === "password" || keySource === "file") && <label className="host-field">{t(draft.authType === "key" ? "Passphrase" : "Password")}<input type="password" value={draft.password ?? ""} onChange={(event) => update({ password: event.target.value })} autoComplete="new-password" placeholder={draft.hasSecret && draft.authType === initial.authType ? t("Saved — leave blank to keep") : t(draft.authType === "key" ? "Optional passphrase" : "Password")} /></label>}
          </section>
        </fieldset>
        {error && <div className="host-error" role="alert">{error}</div>}
        <footer className="host-editor-footer">
          <button className="host-button" type="submit" value="save" disabled={busy}>{t(busy ? "Saving…" : "Save")}</button>
          <button className="host-button host-primary" type="submit" value="connect" disabled={busy || !onConnect}>{t("Save and connect")}</button>
        </footer>
      </form>
    </HostModal>
    {discard && <HostModal titleId={`${id}-discard`} onClose={() => setDiscard(false)}>
      <h2 id={`${id}-discard`}>{t("Discard changes?")}</h2><p>{t("Your unsaved changes will be lost.")}</p>
      <div className="host-dialog-actions"><button autoFocus type="button" className="host-button" onClick={() => setDiscard(false)}>{t("Keep editing")}</button><button type="button" className="host-button host-danger-button" onClick={onClose}>{t("Discard")}</button></div>
    </HostModal>}
  </>;
}

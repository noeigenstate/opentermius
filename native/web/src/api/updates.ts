import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { t } from "../i18n";

export interface UpdateCheck { version: string | null; upToDate: boolean; error: string | null }
export const checkUpdate = () => invoke<UpdateCheck>("check_update");
export const installUpdate = () => invoke<void>("install_update");

/** Mounted by App so the startup check also runs before visiting Settings. */
export function useUpdates(enabled: boolean) {
  const [result, setResult] = useState<UpdateCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState("");
  const running = useRef(false);
  const checkedAutomatically = useRef(false);
  const autoEnabled = useRef(enabled);
  autoEnabled.current = enabled;

  const install = useCallback(async (version: string) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    try {
      const accepted = await ask(t("Install OpenTermius {version}? Installation restarts the app and closes open sessions.", { version }), {
        title: t("Update available"), kind: "warning", okLabel: t("Install update"), cancelLabel: t("Cancel"),
      });
      if (accepted) {
        setInstalling(true);
        await installUpdate();
      }
    } catch (e) { setError(String(e)); }
    finally { running.current = false; setBusy(false); setInstalling(false); }
  }, []);

  const check = useCallback(async (automatic = false) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    let available: string | null = null;
    try {
      const next = await checkUpdate();
      setResult(next);
      setError(next.error ?? "");
      available = next.version;
    } catch (e) { setError(String(e)); }
    finally { running.current = false; setBusy(false); }
    if (automatic && autoEnabled.current && available) await install(available);
  }, [install]);

  useEffect(() => {
    if (!enabled || checkedAutomatically.current) return;
    const timer = window.setTimeout(() => {
      checkedAutomatically.current = true;
      void check(true);
    }, 5000);
    return () => window.clearTimeout(timer);
  }, [enabled, check]);

  return { result, busy, installing, error, check, install };
}

export type Updates = ReturnType<typeof useUpdates>;

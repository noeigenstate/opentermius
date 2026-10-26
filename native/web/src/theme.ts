import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Settings } from "./api/settings";

const REMEMBERED_THEME = "opentermius-theme";

export function restoreTheme() {
  let theme = "light";
  try { if (localStorage.getItem(REMEMBERED_THEME) === "dark") theme = "dark"; } catch {}
  document.documentElement.dataset.theme = theme;
}

function timeMinutes(value: string): number | null {
  return /^([01]\d|2[0-3]):[0-5]\d$/.test(value) ? Number(value.slice(0, 2)) * 60 + Number(value.slice(3)) : null;
}

export function resolveTheme(settings: Pick<Settings, "appearance" | "themeSchedule" | "themeDayStart" | "themeNightStart">, systemDark: boolean, now = new Date()): "light" | "dark" {
  if (!settings.themeSchedule) {
    if (settings.appearance === "system") return systemDark ? "dark" : "light";
    return settings.appearance === "dark" ? "dark" : "light";
  }
  let day = timeMinutes(settings.themeDayStart), night = timeMinutes(settings.themeNightStart);
  if (day === null || night === null || day === night) { day = 7 * 60; night = 19 * 60; }
  const minutes = now.getHours() * 60 + now.getMinutes();
  const light = day < night ? minutes >= day && minutes < night : minutes >= day || minutes < night;
  return light ? "light" : "dark";
}

export function followTheme(settings: Settings) {
  const root = document.documentElement;
  const media = !settings.themeSchedule && settings.appearance === "system" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  function syncBackground() {
    // A maximized Windows 10 window is square too; only Rust knows whether it is opaque.
    if (root.dataset.windowBackground !== "opaque") return;
    const surface = getComputedStyle(root).getPropertyValue("--surface").trim();
    if (surface) void getCurrentWindow().setBackgroundColor(surface).catch(() => {});
  }
  function apply() {
    const theme = resolveTheme(settings, media?.matches ?? false);
    if (root.dataset.theme !== theme) root.dataset.theme = theme;
    try { localStorage.setItem(REMEMBERED_THEME, theme); } catch {}
    syncBackground();
  }
  const observer = new MutationObserver(syncBackground);
  observer.observe(root, { attributes: true, attributeFilter: ["data-window-background"] });
  apply();
  media?.addEventListener("change", apply);
  const scheduled = settings.themeSchedule;
  const timer = scheduled ? window.setInterval(apply, 60_000) : null;
  if (scheduled) { window.addEventListener("focus", apply); document.addEventListener("visibilitychange", apply); }
  return () => {
    observer.disconnect();
    media?.removeEventListener("change", apply);
    if (timer !== null) window.clearInterval(timer);
    if (scheduled) { window.removeEventListener("focus", apply); document.removeEventListener("visibilitychange", apply); }
  };
}

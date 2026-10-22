import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Settings {
  language: "en" | "zh-CN" | "system";
  fontSize: number;
  scrollbackLines: number;
  cursorBlink: boolean;
  copyOnSelect: boolean;
  confirmMultilinePaste: boolean;
  bell: "none" | "visual";
  colorScheme: "default" | "solarized-dark" | "monokai" | "light";
  appearance: "light" | "dark" | "system";
  gpuRendering: boolean;
  glassEffect: boolean;
  keepaliveSeconds: number;
  connectionTimeoutSeconds: number;
  confirmOverwrite: boolean;
  showHiddenFiles: boolean;
  autoUpdate: boolean;
}

export interface AppInfo { version: string; configFolder: string }

export const getSettings = () => invoke<Settings>("get_settings");
export const saveSettings = (settings: Settings) => invoke<Settings>("save_settings", { settings });
export const onSettingsChanged = (handler: (settings: Settings) => void) => listen<Settings>("settings://changed", event => handler(event.payload));
export const getAppInfo = () => invoke<AppInfo>("get_app_info");
export const openConfigFolder = () => invoke<void>("open_config_folder");

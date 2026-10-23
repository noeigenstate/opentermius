import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";
import { Icon } from "../icons";
import { Tabs, type TabsProps } from "./Tabs";

/** Frameless window chrome: the empty area drags the window, double-click maximizes. */
export function TitleBar({ tabs }: { tabs: TabsProps }) {
  const win = getCurrentWindow();
  return (
    <header className="titlebar">
      <div className="titlebar-tabs"><Tabs {...tabs} /></div>
      <div className="titlebar-spacer" data-tauri-drag-region />
      <button className="chrome" data-tooltip={t("Minimize")} onClick={() => win.minimize()} aria-label={t("Minimize")}><Icon name="minimize" /></button>
      <button className="chrome" data-tooltip={t("Maximize")} onClick={() => win.toggleMaximize()} aria-label={t("Maximize")}><Icon name="maximize" size={14} /></button>
      <button className="chrome chrome-close" data-tooltip={t("Close")} onClick={() => win.close()} aria-label={t("Close")}><Icon name="close" /></button>
    </header>
  );
}

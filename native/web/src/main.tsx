import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import "./workspaces.css";
import App from "./App";
import { restoreTheme } from "./theme";

document.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (target instanceof HTMLElement && (target.closest("input, textarea") || target.isContentEditable)) return;
  if (window.getSelection()?.toString()) return;
  event.preventDefault();
});

restoreTheme();
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

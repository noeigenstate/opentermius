import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

interface Hint {
  text: string;
  left: number;
  top: number;
  above: boolean;
  container: Element;
}

/** One delegated tooltip for the whole app, including controls inside native dialogs.
 * No pointer tracking or animation loop: depth and feedback are handled by CSS. */
export function SurfaceEffects() {
  const [hint, setHint] = useState<Hint | null>(null);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let anchor: HTMLElement | null = null;

    function dismiss() {
      clearTimeout(timer);
      if (anchor) {
        const ids = (anchor.getAttribute("aria-describedby") ?? "").split(" ").filter((id) => id && id !== "surface-hint");
        if (ids.length) anchor.setAttribute("aria-describedby", ids.join(" "));
        else anchor.removeAttribute("aria-describedby");
      }
      anchor = null;
      setHint(null);
    }

    function control(target: EventTarget | null) {
      return target instanceof Element ? target.closest<HTMLElement>("[data-tooltip]") : null;
    }

    function show(element: HTMLElement, delay: number) {
      if (element.matches(":disabled, [aria-disabled='true']") || !element.dataset.tooltip) return;
      dismiss();
      anchor = element;
      timer = setTimeout(() => {
        if (!element.isConnected || document.hidden) return;
        const rect = element.getBoundingClientRect();
        const above = rect.bottom > window.innerHeight - 65;
        const ids = element.getAttribute("aria-describedby");
        element.setAttribute("aria-describedby", [ids, "surface-hint"].filter(Boolean).join(" "));
        setHint({
          text: element.dataset.tooltip!,
          left: Math.max(130, Math.min(window.innerWidth - 130, rect.left + rect.width / 2)),
          top: above ? rect.top - 10 : rect.bottom + 10,
          above,
          container: element.closest("dialog[open]") ?? document.body,
        });
      }, delay);
    }

    function enter(event: PointerEvent) {
      if (event.pointerType !== "mouse") return;
      const element = control(event.target);
      if (element && !element.contains(event.relatedTarget as Node | null)) show(element, 450);
    }

    function leave(event: PointerEvent) {
      if (anchor && !anchor.contains(event.relatedTarget as Node | null)) dismiss();
    }

    function focus(event: FocusEvent) {
      const element = control(event.target);
      if (element?.matches(":focus-visible")) show(element, 0);
    }

    function escape(event: KeyboardEvent) {
      if (event.key === "Escape") dismiss();
    }

    function visibility() {
      document.documentElement.dataset.motionPaused = String(document.hidden || !document.hasFocus());
      dismiss();
    }

    document.addEventListener("pointerover", enter);
    document.addEventListener("pointerout", leave);
    document.addEventListener("focusin", focus);
    document.addEventListener("focusout", dismiss);
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("pointercancel", dismiss);
    document.addEventListener("keydown", escape);
    document.addEventListener("scroll", dismiss, true);
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("blur", visibility);
    window.addEventListener("focus", visibility);
    window.addEventListener("resize", dismiss);
    visibility();
    return () => {
      clearTimeout(timer);
      dismiss();
      document.removeEventListener("pointerover", enter);
      document.removeEventListener("pointerout", leave);
      document.removeEventListener("focusin", focus);
      document.removeEventListener("focusout", dismiss);
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("pointercancel", dismiss);
      document.removeEventListener("keydown", escape);
      document.removeEventListener("scroll", dismiss, true);
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("blur", visibility);
      window.removeEventListener("focus", visibility);
      window.removeEventListener("resize", dismiss);
      delete document.documentElement.dataset.motionPaused;
    };
  }, []);

  return hint && createPortal(
    <div id="surface-hint" role="tooltip" className={`surface-hint${hint.above ? " surface-hint-above" : ""}`} style={{ left: hint.left, top: hint.top }}>{hint.text}</div>,
    hint.container,
  );
}

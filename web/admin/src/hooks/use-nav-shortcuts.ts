import { useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ALL_NAV_ITEMS, navLinkProps } from "@/lib/nav";

/** True when the keystroke belongs to a text control and must not be hijacked. */
function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName) || target.closest("[role=dialog]") !== null;
}

const CHORD_WINDOW_MS = 900;

/**
 * `g` then a letter jumps to a screen (`g o` Overview, `g l` Logs, …) — the
 * same chord vocabulary GitHub and Linear use. Ignored while typing, inside
 * a dialog, or with a modifier held. No animation by design: this is a
 * keyboard action people repeat dozens of times a day.
 */
export function useNavShortcuts() {
  const navigate = useNavigate();

  useEffect(() => {
    let armedAt = 0;
    function onKeyDown(event: KeyboardEvent) {
      if (event.metaKey || event.ctrlKey || event.altKey || event.repeat || isTyping(event.target)) return;
      const key = event.key.toLowerCase();
      if (key === "g") {
        armedAt = Date.now();
        return;
      }
      if (armedAt === 0 || Date.now() - armedAt > CHORD_WINDOW_MS) {
        armedAt = 0;
        return;
      }
      armedAt = 0;
      const item = ALL_NAV_ITEMS.find((i) => i.chord === key);
      if (!item) return;
      event.preventDefault();
      void navigate(navLinkProps(item) as never);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate]);
}

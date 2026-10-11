import { useEffect, useRef } from "react";

/**
 * Settings screens that hold a draft: ⌘/Ctrl+S saves it (when it is dirty and
 * valid) and closing the tab with unsaved edits asks first. The handler is
 * kept in a ref so the listeners are bound once, not on every keystroke.
 */
export function useSaveShortcut({ dirty, canSave, onSave }: { dirty: boolean; canSave: boolean; onSave: () => void }) {
  const latest = useRef({ dirty, canSave, onSave });
  useEffect(() => {
    latest.current = { dirty, canSave, onSave };
  });

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "s") return;
      event.preventDefault();
      const { dirty: isDirty, canSave: ok, onSave: save } = latest.current;
      if (isDirty && ok) save();
    }
    function onBeforeUnload(event: BeforeUnloadEvent) {
      if (!latest.current.dirty) return;
      event.preventDefault();
    }
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("beforeunload", onBeforeUnload);
    };
  }, []);
}

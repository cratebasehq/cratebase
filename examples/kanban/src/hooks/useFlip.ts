import { createContext, useContext, useLayoutEffect, useRef } from "react";

/** Shared registry of card DOM nodes, keyed by record id, spanning every
 * column — a card moving *between* columns still needs its old rect
 * measured against its new one. `Board` owns the single instance. */
export const FlipRegistryContext = createContext<Map<string, HTMLElement> | null>(null);

export function useFlipRegistry() {
  const ref = useRef<Map<string, HTMLElement>>(new Map());
  return ref.current;
}

export function useRegisterFlipNode(id: string) {
  const registry = useContext(FlipRegistryContext);
  return (el: HTMLElement | null) => {
    if (!registry) return;
    if (el) registry.set(id, el);
    else registry.delete(id);
  };
}

const FLIP_DURATION_MS = 320;
const FLIP_EASING = "cubic-bezier(0.2, 0.8, 0.2, 1)";

/** Classic FLIP: runs after every render where `key` changes (we pass a
 * string built from each card's id+status+order so any reorder — local
 * drag or a realtime event from another tab — triggers it). Diffs the
 * previously measured rect of every registered card against its
 * freshly-rendered rect and, for anything that moved, plays the delta
 * back as a transform so the card visibly glides into place instead of
 * teleporting. */
export function useFlipOnChange(registry: Map<string, HTMLElement>, key: string) {
  const prevRects = useRef<Map<string, DOMRect>>(new Map());

  useLayoutEffect(() => {
    const nextRects = new Map<string, DOMRect>();
    registry.forEach((el, id) => nextRects.set(id, el.getBoundingClientRect()));

    prevRects.current.forEach((oldRect, id) => {
      const el = registry.get(id);
      const newRect = nextRects.get(id);
      if (!el || !newRect) return;
      const dx = oldRect.left - newRect.left;
      const dy = oldRect.top - newRect.top;
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) return;

      el.style.transition = "none";
      el.style.transform = `translate(${dx}px, ${dy}px)`;
      requestAnimationFrame(() => {
        el.style.transition = `transform ${FLIP_DURATION_MS}ms ${FLIP_EASING}`;
        el.style.transform = "";
      });
    });

    prevRects.current = nextRects;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

// Hand-rolled directly on `@cratebase/client` (not `@cratebase/react`'s
// generic `useRecords`), same reasoning as examples/kanban/src/hooks/useCards.ts:
// drag-and-drop reordering needs an optimistic-then-reconcile write that a
// generic refetch-on-realtime-event hook doesn't give you (refetching
// after every drop would blow away the instant local reorder while the
// request is in flight). Reuses kanban's fractional-index scheme, scoped
// per team+column instead of per fixed status column.
import { useCallback, useEffect, useRef, useState } from "react";
import { cb } from "../cratebase.js";
import type { CardsRecord, CardsUpdate } from "../cratebase-types.js";

const ORDER_STEP = 1000;

function orderForIndex(columnCards: CardsRecord[], index: number): number {
  if (columnCards.length === 0) return ORDER_STEP;
  if (index <= 0) return columnCards[0].order - ORDER_STEP;
  if (index >= columnCards.length) return columnCards[columnCards.length - 1].order + ORDER_STEP;
  return (columnCards[index - 1].order + columnCards[index].order) / 2;
}

function sortByOrder(cards: CardsRecord[]): CardsRecord[] {
  return [...cards].sort((a, b) => a.order - b.order);
}

export function useCards(teamId: string | null) {
  const [cards, setCards] = useState<CardsRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [connected, setConnected] = useState(false);
  const cardsRef = useRef<CardsRecord[]>([]);
  cardsRef.current = cards;

  useEffect(() => {
    if (!teamId) {
      setCards([]);
      return;
    }
    let cancelled = false;

    async function boot() {
      setLoading(true);
      try {
        const result = await cb
          .collection("cards")
          .fullList({ filter: `teamRef = "${teamId}"`, sort: "order", expand: "assigneeRef" });
        if (!cancelled) setCards(result);
      } finally {
        if (!cancelled) setLoading(false);
      }

      try {
        await cb.collection("cards").subscribe(
          "*",
          (event) => {
            const record = event.record;
            setCards((prev) => {
              if (event.action === "delete") return prev.filter((c) => c.id !== record.id);
              const idx = prev.findIndex((c) => c.id === record.id);
              if (idx === -1) return sortByOrder([...prev, record]);
              const next = [...prev];
              next[idx] = record;
              return sortByOrder(next);
            });
          },
          { filter: `teamRef = "${teamId}"`, expand: "assigneeRef" },
        );
        if (!cancelled) setConnected(true);
      } catch (err) {
        console.error("realtime subscribe failed", err);
        if (!cancelled) setConnected(false);
      }
    }

    boot();
    return () => {
      cancelled = true;
    };
  }, [teamId]);

  const createCard = useCallback(
    async (columnId: string, title: string) => {
      if (!teamId) return;
      const columnCards = sortByOrder(cardsRef.current.filter((c) => c.columnRef === columnId));
      const order = orderForIndex(columnCards, columnCards.length);
      const record = await cb.collection("cards").create({ teamRef: teamId, columnRef: columnId, title, order });
      setCards((prev) => (prev.some((c) => c.id === record.id) ? prev : sortByOrder([...prev, record])));
      return record;
    },
    [teamId],
  );

  const deleteCard = useCallback(async (id: string) => {
    const prev = cardsRef.current;
    setCards((cur) => cur.filter((c) => c.id !== id));
    try {
      await cb.collection("cards").delete(id);
    } catch (err) {
      setCards(prev);
      throw err;
    }
  }, []);

  /** Moves `id` into `columnId` at `index` among that column's other
   * cards. Applies optimistically, then persists. */
  const moveCard = useCallback(async (id: string, columnId: string, index: number) => {
    const prevCards = cardsRef.current;
    const dragged = prevCards.find((c) => c.id === id);
    if (!dragged) return;

    const columnCards = sortByOrder(prevCards.filter((c) => c.columnRef === columnId && c.id !== id));
    const order = orderForIndex(columnCards, index);
    if (dragged.columnRef === columnId && dragged.order === order) return;

    setCards((cur) => sortByOrder(cur.map((c) => (c.id === id ? { ...c, columnRef: columnId, order } : c))));
    try {
      await cb.collection("cards").update(id, { columnRef: columnId, order });
    } catch (err) {
      setCards(prevCards);
      throw err;
    }
  }, []);

  const updateCard = useCallback(async (id: string, data: Partial<CardsUpdate>) => {
    return cb.collection("cards").update(id, data);
  }, []);

  return { cards, loading, connected, createCard, deleteCard, moveCard, updateCard };
}

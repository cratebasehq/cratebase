import { useCallback, useEffect, useRef, useState } from "react";
import { CARDS_COLLECTION, cb } from "../cratebase";
import type { CardRecord, Status } from "../types";

const ORDER_STEP = 1000;

/** Fractional-index ordering: computes an `order` value for inserting a
 * card at `index` within `columnCards` (already sorted, and with the
 * dragged card excluded) without renumbering any sibling. Landing between
 * two cards takes the midpoint of their `order`s; landing at either end
 * steps `ORDER_STEP` past the current edge. */
function orderForIndex(columnCards: CardRecord[], index: number): number {
  if (columnCards.length === 0) return ORDER_STEP;
  if (index <= 0) return columnCards[0].order - ORDER_STEP;
  if (index >= columnCards.length) return columnCards[columnCards.length - 1].order + ORDER_STEP;
  return (columnCards[index - 1].order + columnCards[index].order) / 2;
}

function sortByOrder(cards: CardRecord[]): CardRecord[] {
  return [...cards].sort((a, b) => a.order - b.order);
}

export function useCards(enabled: boolean) {
  const [cards, setCards] = useState<CardRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [connected, setConnected] = useState(false);
  const cardsRef = useRef<CardRecord[]>([]);
  cardsRef.current = cards;

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;

    async function boot() {
      setLoading(true);
      try {
        const result = (await cb.collection(CARDS_COLLECTION).fullList({ sort: "order" })) as unknown as CardRecord[];
        if (!cancelled) setCards(result);
      } finally {
        if (!cancelled) setLoading(false);
      }

      try {
        await cb.collection(CARDS_COLLECTION).subscribe("*", (event) => {
          const record = event.record as unknown as CardRecord;
          setCards((prev) => {
            if (event.action === "delete") {
              return prev.filter((c) => c.id !== record.id);
            }
            const idx = prev.findIndex((c) => c.id === record.id);
            if (idx === -1) return sortByOrder([...prev, record]);
            const next = [...prev];
            next[idx] = record;
            return sortByOrder(next);
          });
        });
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
  }, [enabled]);

  const createCard = useCallback(async (status: Status, title: string) => {
    const columnCards = sortByOrder(cardsRef.current.filter((c) => c.status === status));
    const order = orderForIndex(columnCards, columnCards.length);
    const record = (await cb.collection(CARDS_COLLECTION).create({ title, status, order })) as unknown as CardRecord;
    // Optimistic insert, deduped by id: the realtime "create" event for
    // this same write can arrive before this continuation runs, so a
    // blind push would double the card — merge instead, matching the
    // subscribe handler's own id-keyed merge above.
    setCards((prev) => (prev.some((c) => c.id === record.id) ? prev : sortByOrder([...prev, record])));
    return record;
  }, []);

  const deleteCard = useCallback(async (id: string) => {
    const prev = cardsRef.current;
    setCards((cur) => cur.filter((c) => c.id !== id));
    try {
      await cb.collection(CARDS_COLLECTION).delete(id);
    } catch (err) {
      setCards(prev); // revert
      throw err;
    }
  }, []);


  /** Moves `id` to `status` at `index` within that column's cards (the
   * index the dragged card should occupy *after* the move, counted among
   * the other cards already in that column). Applies optimistically, then
   * persists — the realtime "update" event settles every other tab. */
  const moveCard = useCallback(async (id: string, status: Status, index: number) => {
    const prevCards = cardsRef.current;
    const dragged = prevCards.find((c) => c.id === id);
    if (!dragged) return;

    const columnCards = sortByOrder(prevCards.filter((c) => c.status === status && c.id !== id));
    const order = orderForIndex(columnCards, index);
    if (dragged.status === status && dragged.order === order) return;

    setCards((cur) => sortByOrder(cur.map((c) => (c.id === id ? { ...c, status, order } : c))));
    try {
      await cb.collection(CARDS_COLLECTION).update(id, { status, order });
    } catch (err) {
      setCards(prevCards); // revert
      throw err;
    }
  }, []);

  return { cards, loading, connected, createCard, deleteCard, moveCard };
}

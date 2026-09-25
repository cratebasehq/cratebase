import type { CardsRecord } from "../cratebase-types.js";
import { Avatar } from "./Avatar.js";

const LABEL_STYLES: Record<string, string> = {
  bug: "bg-crate-50 text-crate dark:bg-crate-dark/15 dark:text-crate-dark",
  feature: "bg-[#E7EEF7] text-manifest dark:bg-manifest/20 dark:text-manifest-light",
  chore: "bg-paper-200 text-ink/60 dark:bg-ink-600 dark:text-paper/60",
  urgent: "bg-crate text-white",
  design: "bg-[#E7F3EC] text-moss dark:bg-moss/20 dark:text-moss",
};

/** A short, stable "manifest tag" from the record id — purely cosmetic
 * (not a real sequence number), styled like a shipping label reference. */
function shortRef(id: string): string {
  return "CB-" + id.slice(0, 4).toUpperCase();
}

export function CardItem({
  card,
  dragging,
  dimmed,
  onDragStart,
  onDragEnd,
  onOpen,
}: {
  card: CardsRecord;
  dragging: boolean;
  dimmed: boolean;
  onDragStart: (event: React.DragEvent) => void;
  onDragEnd: () => void;
  onOpen: () => void;
}) {
  const assignee = card.expand?.assigneeRef;
  return (
    <li
      data-id={card.id}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onOpen}
      className={
        "cursor-pointer rounded-card border border-slate/50 bg-paper-100 p-3 transition-shadow dark:border-slate-dark dark:bg-ink-600 " +
        (dragging ? "shadow-lift" : "") +
        (dimmed ? " opacity-30" : "")
      }
    >
      <div className="flex items-start justify-between gap-2">
        <p className="text-sm font-medium leading-snug">{card.title}</p>
        <span className="shrink-0 font-mono text-[10px] text-ink/35 dark:text-paper/35">{shortRef(card.id)}</span>
      </div>
      {card.labels && card.labels.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {card.labels.map((l) => (
            <span key={l} className={"chip " + (LABEL_STYLES[l] ?? "bg-paper-200 text-ink/60")}>
              {l}
            </span>
          ))}
        </div>
      )}
      <div className="mt-2 flex items-center justify-between">
        <div className="flex items-center gap-2 text-[11px] text-ink/40 dark:text-paper/40">
          {card.dueAt && <span>{new Date(card.dueAt).toLocaleDateString()}</span>}
        </div>
        {assignee && <Avatar id={assignee.id} name={assignee.name || assignee.email} size={20} />}
      </div>
    </li>
  );
}

import { useState } from "react";
import { cb, useAuth, useRecords } from "../cratebase.js";

export function NotificationsMenu({ onOpenCard }: { onOpenCard: (cardId: string) => void }) {
  const { user } = useAuth();
  const [open, setOpen] = useState(false);
  const { records: notifications } = useRecords("notifications", {
    filter: user ? `userRef = "${user.id}"` : undefined,
    sort: "-created",
    perPage: 20,
    enabled: !!user,
  });
  const unread = notifications.filter((n) => !n.read).length;

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="relative rounded-control border border-slate/60 px-2.5 py-1.5 text-sm dark:border-slate-dark"
      >
        🔔
        {unread > 0 && (
          <span className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-crate text-[10px] text-white">
            {unread}
          </span>
        )}
      </button>
      {open && (
        <div className="absolute right-0 z-30 mt-2 w-80 rounded-panel border border-slate/50 bg-paper-100 p-2 shadow-lift dark:border-slate-dark dark:bg-ink-700">
          {notifications.length === 0 && <p className="p-3 text-sm text-ink/50 dark:text-paper/50">No notifications yet.</p>}
          <ul className="flex flex-col">
            {notifications.map((n) => (
              <li key={n.id}>
                <button
                  type="button"
                  onClick={() => {
                    void cb.collection("notifications").update(n.id, { read: true });
                    onOpenCard(n.cardRef);
                    setOpen(false);
                  }}
                  className={
                    "w-full rounded-control px-3 py-2 text-left text-sm hover:bg-paper-200 dark:hover:bg-ink-600 " +
                    (n.read ? "text-ink/50 dark:text-paper/50" : "font-medium")
                  }
                >
                  {n.message}
                  <span className="block text-[11px] font-normal text-ink/40 dark:text-paper/40">
                    {new Date(n.created).toLocaleString()}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

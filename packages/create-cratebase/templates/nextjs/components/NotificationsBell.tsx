"use client";

import { useState } from "react";
import { useNotifications } from "@/lib/cratebase";

export function NotificationsBell() {
  const [open, setOpen] = useState(false);
  const { items, unreadCount, markRead, markAllRead } = useNotifications();

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="btn-secondary relative !px-3"
        aria-label="Notifications"
      >
        🔔
        {unreadCount > 0 && (
          <span className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-[var(--color-accent)] px-1 text-[10px] font-semibold text-white">
            {unreadCount > 9 ? "9+" : unreadCount}
          </span>
        )}
      </button>

      {open && (
        <div className="card absolute right-0 z-10 mt-2 w-72 !p-0">
          <div className="flex items-center justify-between border-b border-black/10 px-4 py-2 dark:border-white/10">
            <span className="text-sm font-medium">Notifications</span>
            {unreadCount > 0 && (
              <button type="button" onClick={() => markAllRead()} className="text-xs text-[var(--color-accent)]">
                Mark all read
              </button>
            )}
          </div>
          <ul className="max-h-72 divide-y divide-black/10 overflow-y-auto dark:divide-white/10">
            {items.length === 0 && <li className="px-4 py-6 text-center text-sm text-black/40">No notifications yet.</li>}
            {items.map((n) => (
              <li key={n.id} className={`px-4 py-2 text-sm ${n.readAt ? "opacity-50" : ""}`}>
                <button type="button" className="w-full text-left" onClick={() => !n.readAt && markRead(n.id)}>
                  <p className="font-medium">{n.title}</p>
                  {n.body && <p className="text-black/60 dark:text-white/60">{n.body}</p>}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

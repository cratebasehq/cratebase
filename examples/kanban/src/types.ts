// Shared record shapes for the `cards` and `presence` collections. Both are
// provisioned by setup.sh — see its inline schema for the source of truth.

export type Status = "todo" | "in_progress" | "done";

export const STATUSES: Status[] = ["todo", "in_progress", "done"];

export const STATUS_LABEL: Record<Status, string> = {
  todo: "Todo",
  in_progress: "In Progress",
  done: "Done",
};

export interface CardRecord {
  id: string;
  title: string;
  status: Status;
  order: number;
  created: string;
  updated: string;
}

export interface PresenceRecord {
  id: string;
  userId: string;
  name: string;
  lastSeen: string;
  created: string;
  updated: string;
}

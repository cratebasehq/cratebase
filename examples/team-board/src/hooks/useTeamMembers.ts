import { useMemo } from "react";
import { useRecords as useRecordsUntyped } from "@cratebase/react";
import { cb } from "../cratebase.js";
import type { TeamMemberRecord } from "../lib/teams.js";
import type { UsersRecord } from "../cratebase-types.js";

export interface TeamMember {
  id: string;
  name: string;
  email: string;
}

/** Every member of a team, expanded to their `users` row — used for the
 * card detail panel's assignee picker. `_team_members` is a system
 * collection, so (like `useTeams`) this goes through the untyped
 * `useRecords`. */
export function useTeamMembers(teamId: string | null) {
  const { records } = useRecordsUntyped<TeamMemberRecord & { expand?: { teamRef?: unknown; userRef?: UsersRecord } }>(
    cb,
    "_team_members",
    { filter: teamId ? `teamRef = "${teamId}"` : undefined, expand: "userRef", enabled: !!teamId },
  );

  return useMemo<TeamMember[]>(
    () =>
      records
        .filter((m) => m.expand?.userRef)
        .map((m) => ({ id: m.userRef, name: m.expand!.userRef!.name ?? "", email: m.expand!.userRef!.email })),
    [records],
  );
}

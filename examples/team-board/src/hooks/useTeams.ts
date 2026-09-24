import { useEffect, useMemo, useState } from "react";
// `_team_members` is a system collection, not part of the generated
// `Schema` — see src/lib/teams.ts — so this goes through the untyped
// `useRecords` straight from `@cratebase/react` (with an explicit type
// param) rather than the `createCratebaseHooks<Schema>()`-bound one in
// cratebase.ts, which only accepts a name from `Schema`.
import { useRecords as useRecordsUntyped } from "@cratebase/react";
import { cb, useAuth } from "../cratebase.js";
import type { TeamMemberRecord } from "../lib/teams.js";

const ACTIVE_TEAM_KEY = "team-board:activeTeamId";

export interface TeamOption {
  membership: TeamMemberRecord;
  teamId: string;
  teamName: string;
}

/** Every team the signed-in user belongs to (via `_team_members`,
 * expanded to the `_teams` row), plus which one is "active" — persisted
 * in `localStorage` so a reload keeps the same board open. */
export function useTeams() {
  const { user } = useAuth();
  const { records, loading, error } = useRecordsUntyped<TeamMemberRecord>(cb, "_team_members", {
    filter: user ? `userRef = "${user.id}"` : undefined,
    expand: "teamRef",
    enabled: !!user,
    sort: "created",
  });

  const teams = useMemo<TeamOption[]>(
    () =>
      records
        .filter((m) => m.expand?.teamRef)
        .map((m) => ({ membership: m, teamId: m.teamRef, teamName: m.expand!.teamRef!.name })),
    [records],
  );

  const [activeTeamId, setActiveTeamId] = useState<string | null>(() => localStorage.getItem(ACTIVE_TEAM_KEY));

  useEffect(() => {
    if (teams.length === 0) return;
    if (activeTeamId && teams.some((t) => t.teamId === activeTeamId)) return;
    setActiveTeamId(teams[0].teamId);
  }, [teams, activeTeamId]);

  function selectTeam(teamId: string) {
    localStorage.setItem(ACTIVE_TEAM_KEY, teamId);
    setActiveTeamId(teamId);
  }

  const activeTeam = teams.find((t) => t.teamId === activeTeamId) ?? null;

  return { teams, activeTeam, activeTeamId: activeTeam?.teamId ?? null, selectTeam, loading, error };
}

/** Creates a new team (the signed-in user becomes its owner) — see
 * `_teams`' `createRule` (`crates/core/src/collection.rs`): any
 * authenticated user may create one as long as `ownerRef` is their own
 * id. The reactive `settings.teams.enabled` hook
 * (`crates/server/src/teams.rs`) inserts the owner's own `_team_members`
 * row automatically. */
export async function createTeam(name: string, ownerId: string): Promise<void> {
  await cb.collection("_teams").create({ name, ownerRef: ownerId });
}

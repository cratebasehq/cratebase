// `_teams`/`_team_members` are Cratebase system collections
// (crates/core/src/collection.rs) — always present once
// `settings.teams.enabled` is on (see scripts/setup.sh), but excluded
// from `cratebase typegen`'s output (system collections aren't part of a
// project's own schema), so they're typed by hand here instead of
// through the generated `Schema`.
export interface TeamRecord {
  id: string;
  name: string;
  ownerRef: string;
  created: string;
  updated: string;
}

export interface TeamMemberRecord {
  id: string;
  teamRef: string;
  userRef: string;
  role: string;
  created: string;
  updated: string;
  expand?: { teamRef?: TeamRecord };
}

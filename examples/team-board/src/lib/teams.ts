import type { RecordModel } from "@cratebase/client";

// `_teams`/`_team_members` are Cratebase system collections
// (crates/core/src/collection.rs) — always present once
// `settings.teams.enabled` is on (see scripts/setup.sh), but excluded
// from `cratebase typegen`'s output (system collections aren't part of a
// project's own schema), so they're typed by hand here instead of
// through the generated `Schema`. Intersected with `RecordModel` (rather
// than declaring `collectionId`/`collectionName` by hand) for the same
// reason `createCratebaseHooks`'s generated hooks do it for every
// `Schema` collection — see sdk/js/react/src/createHooks.ts.
export type TeamRecord = RecordModel & {
  name: string;
  ownerRef: string;
};

export type TeamMemberRecord = RecordModel & {
  teamRef: string;
  userRef: string;
  role: string;
  expand?: { teamRef?: TeamRecord };
};

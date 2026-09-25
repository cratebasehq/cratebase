import { useState } from "react";
import { useAuth, useRecords } from "../cratebase.js";
import { useTeams, createTeam } from "../hooks/useTeams.js";
import { useCards } from "../hooks/useCards.js";
import { useBoardPresence } from "../hooks/useBoardPresence.js";
import { useSearch } from "../hooks/useSearch.js";
import { Avatar } from "./Avatar.js";
import { Board } from "./Board.js";
import { NotificationsMenu } from "./NotificationsMenu.js";
import { CardDetail } from "./CardDetail.js";

export function Workspace() {
  const { user, signOut } = useAuth();
  const { teams, activeTeam, activeTeamId, selectTeam, loading: teamsLoading } = useTeams();
  const cardsApi = useCards(activeTeamId);
  const { records: columns } = useRecords("columns", {
    filter: activeTeamId ? `teamRef = "${activeTeamId}"` : undefined,
    sort: "order",
    enabled: !!activeTeamId,
  });
  const online = useBoardPresence(activeTeamId, user?.id, user?.name || user?.email);
  const search = useSearch(activeTeamId);
  const [query, setQuery] = useState("");
  const [openCardId, setOpenCardId] = useState<string | null>(null);
  const [showNewTeam, setShowNewTeam] = useState(false);

  if (teamsLoading && teams.length === 0) {
    return <div className="flex min-h-screen items-center justify-center text-sm text-ink/50">Loading your teams…</div>;
  }

  if (teams.length === 0) {
    return <NoTeamsScreen userId={user!.id} onSignOut={() => void signOut()} />;
  }

  return (
    <div className="flex min-h-screen">
      <aside className="flex w-60 shrink-0 flex-col border-r border-slate/50 bg-paper-100 dark:border-slate-dark dark:bg-ink-700">
        <div className="flex items-center gap-2 border-b border-slate/50 px-4 py-4 font-display font-semibold dark:border-slate-dark">
          <CrateMark />
          team-board
        </div>
        <div className="p-3">
          <label className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink/40 dark:text-paper/40">
            Team
          </label>
          <select
            className="input w-full"
            value={activeTeamId ?? ""}
            onChange={(e) => selectTeam(e.target.value)}
          >
            {teams.map((t) => (
              <option key={t.teamId} value={t.teamId}>
                {t.teamName}
              </option>
            ))}
          </select>
          <button type="button" onClick={() => setShowNewTeam(true)} className="mt-2 text-xs text-manifest hover:underline dark:text-manifest-light">
            + New team
          </button>
        </div>
        <nav className="flex-1 px-3 text-sm text-ink/70 dark:text-paper/70">
          <div className="rounded-control bg-paper-200 px-3 py-2 font-medium text-ink dark:bg-ink-600 dark:text-paper">Board</div>
        </nav>
        <div className="flex items-center justify-between border-t border-slate/50 px-4 py-3 dark:border-slate-dark">
          <div className="flex items-center gap-2">
            <Avatar id={user!.id} name={user!.name || user!.email} />
            <span className="truncate text-sm">{user!.name || user!.email}</span>
          </div>
          <button type="button" onClick={() => void signOut()} className="text-xs text-ink/50 hover:text-ink dark:text-paper/50 dark:hover:text-paper">
            Sign out
          </button>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-4 border-b border-slate/50 bg-paper-100 px-6 py-3 dark:border-slate-dark dark:bg-ink-700">
          <h1 className="font-display text-lg font-semibold">{activeTeam?.teamName}</h1>
          <div className="ml-2 flex-1 max-w-md">
            <div className="relative">
              <input
                className="input w-full pr-8"
                placeholder="Semantic search…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void search.search(query);
                  if (e.key === "Escape") {
                    setQuery("");
                    search.clear();
                  }
                }}
              />
              <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-crate" title="AI-powered semantic search">
                ✦
              </span>
            </div>
          </div>
          <div className="flex items-center -space-x-2">
            {online.map((p) => (
              <span key={p.userRef} className="rounded-full ring-2 ring-paper-100 dark:ring-ink-700">
                <Avatar id={p.userRef} name={p.name} size={26} />
              </span>
            ))}
          </div>
          <NotificationsMenu onOpenCard={setOpenCardId} />
        </header>

        <main className="flex-1 overflow-x-auto p-6">
          {activeTeamId && (
            <Board
              teamId={activeTeamId}
              columns={columns}
              cardsApi={cardsApi}
              highlightIds={search.results ? new Set(search.results.map((r) => r.id)) : null}
              searching={search.searching}
              onOpenCard={setOpenCardId}
            />
          )}
        </main>
      </div>

      {openCardId && <CardDetail cardId={openCardId} onClose={() => setOpenCardId(null)} />}
      {showNewTeam && <NewTeamDialog userId={user!.id} onClose={() => setShowNewTeam(false)} />}
    </div>
  );
}

function NoTeamsScreen({ userId, onSignOut }: { userId: string; onSignOut: () => void }) {
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4 px-6 text-center">
      <p className="font-display text-xl font-semibold">You're not on a team yet</p>
      <p className="max-w-sm text-sm text-ink/60 dark:text-paper/60">Create one to get a board.</p>
      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!name.trim()) return;
          setPending(true);
          try {
            await createTeam(name.trim(), userId);
          } finally {
            setPending(false);
          }
        }}
      >
        <input className="input" placeholder="Team name" value={name} onChange={(e) => setName(e.target.value)} />
        <button type="submit" disabled={pending} className="btn-primary">
          Create team
        </button>
      </form>
      <button type="button" onClick={onSignOut} className="text-sm text-ink/40 hover:underline">
        Sign out
      </button>
    </div>
  );
}

function NewTeamDialog({ userId, onClose }: { userId: string; onClose: () => void }) {
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-ink/40 p-4">
      <div className="w-full max-w-sm rounded-panel bg-paper-100 p-6 dark:bg-ink-700">
        <h2 className="font-display text-lg font-semibold">New team</h2>
        <form
          className="mt-4 flex flex-col gap-3"
          onSubmit={async (e) => {
            e.preventDefault();
            if (!name.trim()) return;
            setPending(true);
            try {
              await createTeam(name.trim(), userId);
              onClose();
            } finally {
              setPending(false);
            }
          }}
        >
          <input className="input" autoFocus placeholder="Acme Inc" value={name} onChange={(e) => setName(e.target.value)} />
          <div className="flex justify-end gap-2">
            <button type="button" className="btn-ghost" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" disabled={pending} className="btn-primary">
              Create
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function CrateMark() {
  return (
    <svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden="true">
      <rect x="1" y="1" width="18" height="18" rx="3" stroke="#E8622C" strokeWidth="2" />
      <path d="M1 7h18M7 1v18" stroke="#E8622C" strokeWidth="2" />
    </svg>
  );
}

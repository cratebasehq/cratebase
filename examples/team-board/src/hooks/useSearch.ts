// Semantic search over `cards.embedding` via `?nearestTo=` — works fully
// offline because schema.json configures that field's auto-embed with
// Cratebase's deterministic "echo" provider (no `EMBEDDINGS_BASE_URL`
// needed; see crates/server/src/embeddings.rs).
//
// There's no endpoint to embed arbitrary text on its own — a vector field
// only gets (re)computed as a side effect of a real record write through
// its `sourceField` (`crates/server/src/embeddings.rs::apply_embeddings`).
// So this creates a throwaway `cards` row (`isQuery: true`) to get a query
// vector out of the exact same embedding pipeline every real card's
// `embedding` went through, ranks with `nearestTo`, then deletes it — the
// same round-trip-through-the-real-collection trick
// examples/docmind/pb_hooks/docmind.pb.js uses ("Why a throwaway record"),
// except no server hook is needed here at all: any team member already
// has `create`/`delete` on `cards` (schema.json), and the throwaway row
// never appears in any client's list *or realtime feed*, including its
// own creator's — `cards`' `listRule`/`viewRule` excludes `isQuery = true`
// rows, and realtime delivery is gated by that same rule.
import { useCallback, useState } from "react";
import { cb } from "../cratebase.js";
import type { CardsRecord } from "../cratebase-types.js";

export function useSearch(teamId: string | null) {
  const [results, setResults] = useState<CardsRecord[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const search = useCallback(
    async (query: string) => {
      if (!teamId || !query.trim()) {
        setResults(null);
        return;
      }
      setSearching(true);
      setError(null);
      let temp: CardsRecord | undefined;
      try {
        temp = await cb.collection("cards").create({ teamRef: teamId, title: query, isQuery: true, order: 0 });
        // NB: `cb.vector.nearestTo` is a bare re-export of the standalone
        // `nearestTo(sender, collection, field, to, options)` helper
        // (sdk/js/client/src/index.ts), not pre-bound to this client —
        // `cb` itself has to be passed as the first argument (it
        // implements the minimal `Sender` shape via `.send()`), or the
        // string "cards" silently becomes `sender` and the call breaks
        // confusingly. Worth documenting since `cb.vector.nearestTo(...)`
        // reads like every other bound `cb.x.y()` call in this SDK and
        // isn't.
        const nearest = await cb.vector.nearestTo<CardsRecord>(cb, "cards", "embedding", temp.embedding as unknown as number[], {
          limit: 10,
          filter: `teamRef = "${teamId}" && isQuery = false && id != "${temp.id}"`,
        });
        setResults(nearest.items);
      } catch (err) {
        setError(err);
        setResults(null);
      } finally {
        if (temp) void cb.collection("cards").delete(temp.id).catch(() => {});
        setSearching(false);
      }
    },
    [teamId],
  );

  const clear = useCallback(() => setResults(null), []);

  return { results, searching, error, search, clear };
}

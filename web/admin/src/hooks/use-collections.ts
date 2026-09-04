import { useQuery } from "@tanstack/react-query";
import { cb } from "@/lib/api";

export function useCollections() {
  return useQuery({
    queryKey: ["collections"],
    // Every consumer wants the full list (sidebar, command palette, name
    // uniqueness checks) — `getList()`'s default 30-per-page would silently
    // truncate past that.
    queryFn: () => cb.collections.getFullList(),
  });
}

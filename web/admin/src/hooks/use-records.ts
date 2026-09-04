import { useMutation, useQuery, useQueryClient, type QueryKey } from "@tanstack/react-query";
import type { ListResult, RecordModel } from "pocketbase";
import { ClientResponseError } from "pocketbase";
import { cb } from "@/lib/api";

/** The page sizes the grid offers. The server caps `perPage` at 1000; 500
 * is the largest size that still renders as one virtualized page without
 * the request itself becoming the thing you wait on. */
export const PAGE_SIZES = [25, 50, 100, 200, 500] as const;
export const DEFAULT_PAGE_SIZE = 50;

export interface RecordsQuery {
  page: number;
  perPage: number;
  /** A PocketBase filter expression, or "" for none. */
  filter: string;
  /** A `sort` param exactly as the server takes it (`-created`, `author.name`, `@random`). */
  sort: string;
  /** Comma-separated relation paths to expand, or "" for none. */
  expand: string;
  /** Skip the `COUNT(*)`. The server answers `totalItems: -1` and the query
   * measurably speeds up, so this is on whenever the total isn't on screen. */
  skipTotal: boolean;
}

/** Every records query for a collection shares this prefix so one
 * `invalidateQueries`/`setQueriesData` call reaches all of its pages. */
export function recordsKey(collectionName: string, query?: RecordsQuery): QueryKey {
  return query ? ["records", collectionName, query] : ["records", collectionName];
}

export function useRecords(collectionName: string, query: RecordsQuery) {
  return useQuery({
    queryKey: recordsKey(collectionName, query),
    queryFn: () =>
      cb.collection(collectionName).getList<RecordModel>(query.page, query.perPage, {
        filter: query.filter || undefined,
        sort: query.sort || undefined,
        expand: query.expand || undefined,
        skipTotal: query.skipTotal || undefined,
        // The SDK auto-cancels same-path requests, which surfaces here as a
        // thrown "autocancelled" error the moment two pages are in flight.
        // React Query already keys responses by query key, so ordering is
        // safe without it.
        requestKey: null,
      }),
    enabled: collectionName.length > 0,
    placeholderData: (previous) => previous,
    // A filter the server rejects is a 400 that won't get better by asking
    // again — surface it immediately instead of after three retries.
    retry: (count, error) => !(error instanceof ClientResponseError && error.status < 500) && count < 2,
  });
}

type ListSnapshot = [QueryKey, ListResult<RecordModel> | undefined][];

/** Patch every cached page of a collection in place. Used by the optimistic
 * mutations below so an inline cell edit lands on the next frame instead of
 * a full round trip later. */
function patchLists(
  queryClient: ReturnType<typeof useQueryClient>,
  collectionName: string,
  apply: (list: ListResult<RecordModel>) => ListResult<RecordModel>,
): void {
  queryClient.setQueriesData<ListResult<RecordModel>>({ queryKey: recordsKey(collectionName) }, (prev) =>
    prev ? apply(prev) : prev,
  );
}

export function useRecordMutations(collectionName: string) {
  const queryClient = useQueryClient();

  function snapshot(): ListSnapshot {
    return queryClient.getQueriesData<ListResult<RecordModel>>({ queryKey: recordsKey(collectionName) });
  }

  function restore(previous: ListSnapshot | undefined) {
    for (const [key, value] of previous ?? []) queryClient.setQueryData(key, value);
  }

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: recordsKey(collectionName) });
  }

  const create = useMutation({
    mutationFn: (data: Record<string, unknown> | FormData) => cb.collection(collectionName).create(data),
    onSuccess: invalidate,
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> | FormData }) =>
      cb.collection(collectionName).update(id, data),
    // Optimistic: write the new value into every cached page, remember the
    // old pages, and put them back if the server says no.
    onMutate: async ({ id, data }) => {
      if (data instanceof FormData) return { previous: undefined };
      await queryClient.cancelQueries({ queryKey: recordsKey(collectionName) });
      const previous = snapshot();
      patchLists(queryClient, collectionName, (list) => ({
        ...list,
        items: list.items.map((item) => (item.id === id ? { ...item, ...data } : item)),
      }));
      return { previous };
    },
    onError: (_error, _variables, context) => restore(context?.previous),
    onSettled: invalidate,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection(collectionName).delete(id),
    onMutate: async (id) => {
      await queryClient.cancelQueries({ queryKey: recordsKey(collectionName) });
      const previous = snapshot();
      patchLists(queryClient, collectionName, (list) => {
        const items = list.items.filter((item) => item.id !== id);
        const removed = list.items.length - items.length;
        return { ...list, items, totalItems: list.totalItems < 0 ? list.totalItems : list.totalItems - removed };
      });
      return { previous };
    },
    onError: (_error, _id, context) => restore(context?.previous),
    onSettled: invalidate,
  });

  /**
   * Bulk delete through `POST /api/batch` — one transactional request for
   * the whole selection instead of N round trips. The endpoint is gated by
   * `settings.batch.enabled` and capped at `settings.batch.maxRequests`, so
   * the selection is chunked and the caller is told when batch is off
   * rather than silently falling back to a slow loop.
   */
  const removeMany = useMutation({
    mutationFn: async ({ ids, chunkSize }: { ids: string[]; chunkSize: number }) => {
      for (let i = 0; i < ids.length; i += chunkSize) {
        const batch = cb.createBatch();
        for (const id of ids.slice(i, i + chunkSize)) batch.collection(collectionName).delete(id);
        await batch.send();
      }
      return ids;
    },
    onMutate: async ({ ids }) => {
      await queryClient.cancelQueries({ queryKey: recordsKey(collectionName) });
      const previous = snapshot();
      const doomed = new Set(ids);
      patchLists(queryClient, collectionName, (list) => {
        const items = list.items.filter((item) => !doomed.has(item.id));
        const removed = list.items.length - items.length;
        return { ...list, items, totalItems: list.totalItems < 0 ? list.totalItems : list.totalItems - removed };
      });
      return { previous };
    },
    onError: (_error, _variables, context) => restore(context?.previous),
    onSettled: invalidate,
  });

  return { create, update, remove, removeMany };
}

export interface BatchCapability {
  enabled: boolean;
  maxRequests: number;
}

/** `POST /api/batch` is off by default and capped per-request, and the
 * selection toolbar has to say so rather than firing a request that 403s. */
export function useBatchCapability() {
  return useQuery<BatchCapability>({
    queryKey: ["settings", "batch"],
    queryFn: async () => {
      const settings = (await cb.settings.getAll()) as { batch?: { enabled?: boolean; maxRequests?: number } };
      return {
        enabled: settings.batch?.enabled === true,
        maxRequests: Math.max(1, Number(settings.batch?.maxRequests ?? 50)),
      };
    },
    staleTime: 5 * 60_000,
    retry: false,
  });
}

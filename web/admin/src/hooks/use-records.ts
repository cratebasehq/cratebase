import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cb } from "@/lib/api";

export function useRecords(collectionName: string, page: number, filter: string, sort: string) {
  return useQuery({
    queryKey: ["records", collectionName, page, filter, sort],
    queryFn: () => cb.collection(collectionName).getList(page, 25, { filter: filter || undefined, sort }),
    enabled: collectionName.length > 0,
    placeholderData: (previous) => previous,
  });
}

export function useRecordMutations(collectionName: string) {
  const queryClient = useQueryClient();

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["records", collectionName] });
  }

  const create = useMutation({
    mutationFn: (data: Record<string, unknown> | FormData) => cb.collection(collectionName).create(data),
    onSuccess: invalidate,
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> | FormData }) =>
      cb.collection(collectionName).update(id, data),
    onSuccess: invalidate,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection(collectionName).delete(id),
    onSuccess: invalidate,
  });

  return { create, update, remove };
}

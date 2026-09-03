import { useQuery } from "@tanstack/react-query";
import { cb } from "@/lib/api";

export function useCollections() {
  return useQuery({
    queryKey: ["collections"],
    queryFn: () => cb.collections.getList(),
  });
}

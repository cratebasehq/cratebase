import { useCallback, useState } from "react";

/**
 * A piece of state that survives a reload, for view preferences that are
 * nobody else's business — row height, whether to run `COUNT(*)`.
 *
 * Anything worth sharing (page, sort, filter) belongs in the URL instead;
 * anything the server has an opinion about belongs on the server. This is
 * the third category.
 */
export function useLocalState<T>(key: string, fallback: T, isValid: (value: unknown) => value is T) {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      if (raw === null) return fallback;
      const parsed: unknown = JSON.parse(raw);
      return isValid(parsed) ? parsed : fallback;
    } catch {
      return fallback;
    }
  });

  const set = useCallback(
    (next: T) => {
      setValue(next);
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        /* preference-only — a full or disabled store is not worth an error */
      }
    },
    [key],
  );

  return [value, set] as const;
}

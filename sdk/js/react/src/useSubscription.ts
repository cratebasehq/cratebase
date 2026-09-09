/** Thin, stateless wrapper over `CollectionService.subscribe` — for
 * callers who want raw realtime events without `useRecords`'/`useRecord`'s
 * list-merging opinion. Subscribes on mount, unsubscribes on unmount or
 * when `client`/`collectionName`/`topic` change, and keeps `callback`
 * current via a ref so an inline arrow function passed at every render
 * does not force a resubscribe. */

import { useEffect, useRef } from "react";
import type { CratebaseClient, RecordModel, SubscribeOptions } from "@cratebase/client";

export type SubscriptionEvent<T extends RecordModel = RecordModel> = {
  action: "create" | "update" | "delete";
  record: T;
};

export function useSubscription<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  topic: string,
  callback: (event: SubscriptionEvent<T>) => void,
  options: SubscribeOptions = {},
): void {
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  const optionsKey = JSON.stringify(options);

  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    let cancelled = false;

    client
      .collection(collectionName)
      .subscribe(
        topic,
        (event) => callbackRef.current(event as SubscriptionEvent<T>),
        options as SubscribeOptions,
      )
      .then((unsub) => {
        if (cancelled) {
          unsub();
          return;
        }
        unsubscribe = unsub;
      })
      .catch((err) => console.error(`useSubscription: subscribe to ${collectionName}/${topic} failed`, err));

    return () => {
      cancelled = true;
      unsubscribe?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, topic, optionsKey]);
}

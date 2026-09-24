/** Thin, stateless wrapper over `CollectionService.subscribe` — for
 * callers who want raw realtime events without `useRecords`'/`useRecord`'s
 * list-merging opinion. Subscribes on mount, unsubscribes on unmount or
 * when `client`/`collectionName`/`topic` change, and keeps `callback`
 * current via a ref so an inline arrow function passed at every render
 * does not force a resubscribe. */

import { useEffect, useRef } from "react";
import type { CratebaseClient, RecordModel, SubscribeOptions } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export type SubscriptionEvent<T extends RecordModel = RecordModel> = {
  action: "create" | "update" | "delete";
  record: T;
};

export function useSubscription<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  topic: string,
  callback: (event: SubscriptionEvent<T>) => void,
  options?: SubscribeOptions,
): void;
export function useSubscription<T extends RecordModel = RecordModel>(
  collectionName: string,
  topic: string,
  callback: (event: SubscriptionEvent<T>) => void,
  options?: SubscribeOptions,
): void;
export function useSubscription<T extends RecordModel = RecordModel>(
  arg0: CratebaseClient<any> | string,
  arg1: string,
  arg2: string | ((event: SubscriptionEvent<T>) => void),
  arg3?: SubscribeOptions | ((event: SubscriptionEvent<T>) => void),
  arg4?: SubscribeOptions,
): void {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : (arg1 as string))!;
  const topic = typeof arg0 === "string" ? (arg1 as string) : (arg2 as string);
  const callback = (typeof arg0 === "string" ? arg2 : arg3) as (event: SubscriptionEvent<T>) => void;
  const options: SubscribeOptions = ((typeof arg0 === "string" ? arg3 : arg4) as SubscribeOptions | undefined) ?? {};

  const client = useResolvedClient(explicitClient);
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
        JSON.parse(optionsKey) as SubscribeOptions,
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

import AsyncStorage from "@react-native-async-storage/async-storage";
import { createClient, AsyncAuthStore } from "@cratebase/client";
import { createCratebaseHooks } from "@cratebase/react";
import type { Schema, SchemaCreate, SchemaUpdate } from "../cratebase-types";

const CRATEBASE_URL = process.env.EXPO_PUBLIC_CRATEBASE_URL ?? "http://localhost:8090";
const STORAGE_KEY = "cratebase_auth";

// React Native's persistence API (AsyncStorage) is itself async, unlike the
// browser's synchronous localStorage — AsyncAuthStore is @cratebase/client's
// store implementation for exactly that. `authStore.whenReady()` resolves
// once `initial` has hydrated the store; `useAuth`'s `isLoading` covers the
// same gap for UI, so most screens never need to call it directly.
export const authStore = new AsyncAuthStore({
  save: (serialized) => AsyncStorage.setItem(STORAGE_KEY, serialized),
  clear: () => AsyncStorage.removeItem(STORAGE_KEY),
  initial: AsyncStorage.getItem(STORAGE_KEY),
});

export const cb = createClient<Schema, SchemaCreate, SchemaUpdate>(CRATEBASE_URL, { authStore });

export const { useRecords, useRecord, useInfiniteRecords, useMutation, useAuth, useCratebase, useSubscription } =
  createCratebaseHooks<Schema>();

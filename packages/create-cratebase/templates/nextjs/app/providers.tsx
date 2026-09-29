"use client";

import type { ReactNode } from "react";
import { CratebaseProvider } from "@cratebase/react";
import { cb } from "@/lib/cratebase";

// `CratebaseProvider` and every hook in `@cratebase/react` use React context
// and `useSyncExternalStore`, so this needs its own "use client" boundary —
// the root layout (app/layout.tsx) stays a server component and just renders
// this around `children`. See the Next.js note in sdk/js/react/README.md.
export function Providers({ children }: { children: ReactNode }) {
  return <CratebaseProvider client={cb}>{children}</CratebaseProvider>;
}

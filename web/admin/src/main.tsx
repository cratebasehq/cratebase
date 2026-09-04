import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { MutationCache, QueryCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { ThemeProvider } from "next-themes";
import { toast } from "sonner";
import { AppErrorBoundary } from "@/components/app-states";
import { Toaster } from "@/components/ui/sonner";
import { describeFailure, isSessionExpired, signOut } from "@/lib/api";
import { router } from "@/router";
import "./index.css";

/**
 * One place decides what a failed request means.
 *
 * A 401 is the session expiring: clear it and go to /login once, rather than
 * letting every in-flight query raise its own toast forever (the behaviour
 * the audit found). Everything else gets one honest toast built from the
 * `{status, message, data}` envelope.
 */
let redirectingToLogin = false;

function handleGlobalError(error: unknown, silent = false) {
  if (isSessionExpired(error)) {
    if (redirectingToLogin) return;
    redirectingToLogin = true;
    signOut();
    queryClient.cancelQueries().finally(() => {
      queryClient.clear();
      toast.error("Your session expired", { description: "Sign in again to continue." });
      void router.navigate({ to: "/login" }).finally(() => {
        redirectingToLogin = false;
      });
    });
    return;
  }

  if (silent) return;
  const failure = describeFailure(error);
  toast.error(failure.title, failure.detail ? { description: failure.detail } : undefined);
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      // Retrying a 4xx just delays the error; retry only what might be a blip.
      retry: (failureCount, error) => !isSessionExpired(error) && failureCount < 1,
    },
  },
  queryCache: new QueryCache({
    // Queries render their own error states, so a background refetch failure
    // must not also shout in a toast — only the session check runs here.
    onError: (error) => handleGlobalError(error, true),
  }),
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) => {
      // A mutation with its own onError has already told the user something
      // specific; don't say it twice.
      handleGlobalError(error, Boolean(mutation.options.onError));
    },
  }),
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppErrorBoundary>
      <ThemeProvider attribute="class" defaultTheme="dark" enableSystem disableTransitionOnChange>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
          <Toaster position="bottom-right" />
        </QueryClientProvider>
      </ThemeProvider>
    </AppErrorBoundary>
  </StrictMode>,
);

import { StrictMode, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { useAuthStore } from "./stores/auth-store";
import { useConsentStore } from "./stores/consent-store";
import { usePublicConfig } from "./hooks/use-public-config";
import { initTelemetry, identify as telemetryIdentify } from "./lib/telemetry";
import { ConsentBanner } from "./components/consent-banner";
import "./app.css";

// Clear the chunk-reload guard on successful app bootstrap.
// This ensures future deploys can auto-reload again.
sessionStorage.removeItem("nyxid_chunk_reload");

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 60 * 1000,
      retry: (failureCount, error) => {
        if (
          error &&
          typeof error === "object" &&
          "status" in error &&
          (error as { status: number }).status === 401
        ) {
          return false;
        }
        return failureCount < 3;
      },
    },
  },
});

function Root() {
  const [ready, setReady] = useState(false);
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const consentEnabled = useConsentStore((s) => s.enabled);
  // Runtime telemetry config. TanStack Query caches this with
  // `staleTime: Infinity` (see hooks/use-public-config.ts) — fetched
  // once per app session, never refetched on effect re-runs or
  // re-renders. Every other consumer of `usePublicConfig` reads from
  // the same cache entry, so telemetry wiring adds zero extra network.
  const { data: publicConfig } = usePublicConfig();

  useEffect(() => {
    useAuthStore
      .getState()
      .checkAuth()
      .finally(() => setReady(true));
  }, []);

  // Initialize telemetry once:
  //   1. auth has resolved (we know who the user is, if any)
  //   2. public config has landed (we know the DSN / host / share-back)
  //   3. consent is granted
  // If config is still in-flight, skip — we'll re-run when `publicConfig`
  // updates. Running before config lands would initialize telemetry in
  // default-off mode and require a second `initTelemetry` call to flip
  // on, which the internal `inited` guard in lib/telemetry.ts blocks.
  useEffect(() => {
    if (!ready || !publicConfig) return;
    initTelemetry({
      dsn: publicConfig.telemetry_dsn,
      host: publicConfig.telemetry_host,
      shareBack: publicConfig.telemetry_share_analytics === true,
      consent: consentEnabled,
    });
    // If we restored an existing session, identify immediately so
    // post-boot pageviews attribute to `user_id` rather than the anon id.
    const user = useAuthStore.getState().user;
    if (isAuthenticated && user?.id) {
      telemetryIdentify(user.id);
    }
  }, [ready, publicConfig, consentEnabled, isAuthenticated]);

  // When auth resolves, redirect as needed:
  // - Authenticated user on landing → dashboard
  // - Unauthenticated user on protected route → login
  useEffect(() => {
    if (!ready) return;
    const path = window.location.pathname;
    if (isAuthenticated && path === "/") {
      router.navigate({ to: "/dashboard" });
    } else if (!isAuthenticated) {
      const isPublicRoute =
        path === "/" ||
        path === "/login" ||
        path === "/register" ||
        path === "/privacy" ||
        path.startsWith("/error") ||
        path.startsWith("/oauth-consent") ||
        path === "/cli-auth";
      if (!isPublicRoute) {
        router.navigate({ to: "/login" });
      }
    }
  }, [ready, isAuthenticated]);

  // Only block rendering on auth for protected routes.
  // Public routes (landing, login, register, etc.) render immediately.
  if (!ready) {
    const path = window.location.pathname;
    const isPublicRoute =
      path === "/" ||
      path === "/login" ||
      path === "/register" ||
      path === "/privacy" ||
      path.startsWith("/error") ||
      path.startsWith("/oauth-consent") ||
      path === "/cli-auth";
    if (!isPublicRoute) return null;
  }

  return (
    <>
      <RouterProvider router={router} />
      <ConsentBanner />
    </>
  );
}

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element not found");
}

// QueryClientProvider must wrap Root — `usePublicConfig()` (and every
// other TanStack Query hook used inside Root) throws without a provider
// above it in the tree.
createRoot(rootElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <Root />
    </QueryClientProvider>
  </StrictMode>,
);

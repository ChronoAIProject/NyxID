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

// Paths that should render without first resolving the auth
// session. Includes the landing/login/register pages users may
// hit before authenticating, and `/cli/pair`: that route is the
// remote-pairing target and must render for unauthenticated
// visitors so `CliPairPage` can redirect to `/login` with a
// `return_to` carrying `?code=...` intact. Routing to bare
// `/login` from here would drop the query string and strand the
// pairing.
function isPublicPath(path: string): boolean {
  return (
    path === "/" ||
    path === "/login" ||
    path === "/register" ||
    path === "/privacy" ||
    path === "/terms" ||
    path === "/blog" ||
    path.startsWith("/blog/") ||
    path.startsWith("/preview/") ||
    path.startsWith("/error") ||
    path.startsWith("/oauth-consent") ||
    path === "/cli-auth" ||
    path === "/cli/pair"
  );
}

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
  const consentAsked = useConsentStore((s) => s.asked);
  const consentEnabled = useConsentStore((s) => s.enabled);

  // Runtime telemetry config. Cached with `staleTime: Infinity`
  // (see hooks/use-public-config.ts), so fetched at most once per
  // session and shared with every other consumer of the hook.
  //
  // Skipped entirely when the user has explicitly DECLINED the
  // consent banner (asked=true, enabled=false). In that case no
  // telemetry will ever initialize, so fetching the config would
  // be a wasted round-trip and — more importantly — would violate
  // the default-off "byte-identical to pre-telemetry" contract
  // on a deploy where the backend sends an empty config. Callers
  // on pages that genuinely need public config (settings, login,
  // MCP tabs) still fetch it via their own hook invocations.
  const telemetryMightInit = !consentAsked || consentEnabled;
  const { data: publicConfig } = usePublicConfig({ enabled: telemetryMightInit });

  useEffect(() => {
    useAuthStore
      .getState()
      .checkAuth()
      .finally(() => {
        setReady(true);
      });
  }, []);

  // Initialize telemetry once:
  //   1. auth has resolved (we know who the user is, if any)
  //   2. public config has landed (we know the DSN / host / share-back)
  //   3. consent is granted
  // If the fetch was skipped because the user declined, `publicConfig`
  // stays undefined forever and we simply never initialize — which is
  // the correct outcome.
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
      if (!isPublicPath(path)) {
        router.navigate({ to: "/login" });
      }
    }
  }, [ready, isAuthenticated]);

  // Only block rendering on auth for protected routes.
  // Public routes (landing, login, register, etc.) render immediately.
  if (!ready) {
    if (!isPublicPath(window.location.pathname)) return null;
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

import { StrictMode, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { useAuthStore } from "./stores/auth-store";
import { useConsentStore } from "./stores/consent-store";
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

  useEffect(() => {
    useAuthStore
      .getState()
      .checkAuth()
      .finally(() => setReady(true));
  }, []);

  // Initialize telemetry once auth resolves AND consent is granted.
  // `initTelemetry` is internally idempotent (StrictMode-safe).
  // When consent is false the call is a no-op, so flipping the toggle
  // off at runtime relies on `reset()` + page reload to fully unwind.
  useEffect(() => {
    if (!ready) return;
    initTelemetry({
      dsn: import.meta.env.VITE_TELEMETRY_DSN,
      host: import.meta.env.VITE_TELEMETRY_HOST,
      shareBack: import.meta.env.VITE_NYXID_SHARE_ANALYTICS === "true",
      consent: consentEnabled,
    });
    // If we restored an existing session, identify immediately so
    // post-boot pageviews attribute to `user_id` rather than the anon id.
    const user = useAuthStore.getState().user;
    if (isAuthenticated && user?.id) {
      telemetryIdentify(user.id);
    }
  }, [ready, consentEnabled, isAuthenticated]);

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
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <ConsentBanner />
    </QueryClientProvider>
  );
}

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element not found");
}

createRoot(rootElement).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);

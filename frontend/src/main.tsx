import { StrictMode, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { useAuthStore } from "./stores/auth-store";
import "./app.css";

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

  useEffect(() => {
    useAuthStore
      .getState()
      .checkAuth()
      .finally(() => setReady(true));
  }, []);

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

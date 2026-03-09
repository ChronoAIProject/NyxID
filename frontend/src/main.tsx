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
    useAuthStore.getState().checkAuth().finally(() => setReady(true));
  }, []);

  // When auth state is cleared (e.g. after failed token refresh),
  // redirect to login so the user isn't stuck on a broken dashboard.
  useEffect(() => {
    if (ready && !isAuthenticated) {
      const path = window.location.pathname;
      const isPublicRoute =
        path === "/login" ||
        path === "/register" ||
        path === "/privacy" ||
        path.startsWith("/error") ||
        path.startsWith("/oauth-consent");
      if (!isPublicRoute) {
        router.navigate({ to: "/login" });
      }
    }
  }, [ready, isAuthenticated]);

  if (!ready) return null;

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

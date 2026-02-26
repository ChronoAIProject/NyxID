import { StrictMode, useState, useCallback, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StatusBar, Style } from "@capacitor/status-bar";
import { router } from "./router";
import { useAuthStore } from "./stores/auth-store";
import { isNative } from "./lib/platform";
import { SplashScreen } from "./components/splash-screen";
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
  const [showSplash, setShowSplash] = useState(true);

  useEffect(() => {
    useAuthStore.getState().checkAuth().finally(() => setReady(true));
  }, []);

  const handleSplashFinish = useCallback(() => {
    setShowSplash(false);
  }, []);

  return (
    <>
      {showSplash && <SplashScreen onFinish={handleSplashFinish} minDuration={ready ? 800 : 2400} />}
      {ready && (
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      )}
    </>
  );
}

function init() {
  if (isNative) {
    void StatusBar.setStyle({ style: Style.Dark });
    void StatusBar.setBackgroundColor({ color: "#06060a" });
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
}

init();

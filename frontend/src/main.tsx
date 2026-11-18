import "./lib/zod-csp";
import { appLanguageReady } from "./i18n";
import "./lib/platform-personal-handoff";
import "./lib/platform-onboarding-handoff";
import { preserveTelegramClaimForLogin } from "./lib/telegram-claim-handoff";
import { StrictMode, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { useAuthStore } from "./stores/auth-store";
import { useConsentStore } from "./stores/consent-store";
import { usePublicConfig } from "./hooks/use-public-config";
import { initTelemetry, identify as telemetryIdentify } from "./lib/telemetry";
import { isPublicPath } from "./lib/public-paths";
import { isAccountEvidencePending, subscribeAssistantIdentity } from "./lib/assistant/identity";
import { createAccountQueryScope } from "./lib/account-query-cache";
import { installAccountSessionSync } from "./lib/account-session-sync";
import { AccountVerificationStatus } from "./components/account-verification-status";
import { AccountBoundary } from "./components/account-boundary";
import { ConsentBanner } from "./components/consent-banner";
import { recoverFromAssetError } from "./lib/chunk-recovery";
import { shouldRetryQuery } from "./lib/query-retry";
import { BuildUpdateBanner } from "./components/build-update-banner";
import "./app.css";
import { installNyxbotErrorFallback } from "./lib/assistant/error-telemetry";

import { installCrashReporting, crashRootOptions } from "./lib/crash-reporting/install";
import { subscribeCrashPermission } from "./lib/crash-reporting/permission-refresh";
installCrashReporting();
installNyxbotErrorFallback();

// Vite raises this from `__vitePreload` when a chunk (or its CSS) cannot be
// fetched, *before* the failure becomes a React.lazy rejection — so it catches
// dependency-preload failures that never reach a render boundary at all.
//
// Cancelling suppresses the rethrow, which is only safe once we have committed
// to navigating away. On "exhausted" we deliberately let it propagate so the
// route error component can render a message instead of leaving a dead screen.
window.addEventListener("vite:preloadError", (event) => {
  if (recoverFromAssetError() === "reloading") {
    event.preventDefault();
  }
});

const accountQueries = createAccountQueryScope({
  defaultOptions: { queries: { staleTime: 60 * 1000, retry: shouldRetryQuery } },
}, subscribeCrashPermission);
installAccountSessionSync();

// Dev-only handle for the e2e harness (frontend/e2e/): flow specs assert on
// cache slots nothing renders yet (e.g. the per-conversation turn episode).
// Stripped from production builds with the rest of the DEV branch.
if (import.meta.env.DEV) {
  (window as { __nyxQueryClient?: QueryClient }).__nyxQueryClient = accountQueries.getClient();
  subscribeAssistantIdentity(() => {
    (window as { __nyxQueryClient?: QueryClient }).__nyxQueryClient = accountQueries.getClient();
  });
  // Use the instances mounted below, not a fresh module import: Vite's HMR
  // timestamps can otherwise give browser drivers a second, idle router/store.
  (window as { __nyxApp?: { router: typeof router; authStore: typeof useAuthStore } }).__nyxApp = { router, authStore: useAuthStore };
}

function isLoginApprovalPage() {
  return ["/login/agent-key", "/login/device", "/login/code"].includes(
    window.location.pathname,
  );
}

function AccountRuntime({ ready }: { ready: boolean }) {
  const queryClient = useQueryClient();
  const [agentKeyLogin, setAgentKeyLogin] = useState(isLoginApprovalPage);
  useEffect(
    () =>
      router.subscribe("onResolved", () => {
        setAgentKeyLogin(isLoginApprovalPage());
      }),
    [],
  );
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const confirmedUser = useAuthStore((s) => s.user);
  const telemetryAccountId = confirmedUser?.id;
  useEffect(() => {
    if (confirmedUser && !isAccountEvidencePending()) queryClient.setQueryData(["user", "me"], confirmedUser);
  }, [confirmedUser, queryClient]);
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
  const { data: publicConfig } = usePublicConfig({
    enabled: telemetryMightInit && !agentKeyLogin,
  });

  // Initialize telemetry once:
  //   1. auth has resolved (we know who the user is, if any)
  //   2. public config has landed (we know the DSN / host / share-back)
  //   3. consent is granted
  // If the fetch was skipped because the user declined, `publicConfig`
  // stays undefined forever and we simply never initialize — which is
  // the correct outcome.
  useEffect(() => {
    if (!ready || !publicConfig || agentKeyLogin) return;
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
  }, [ready, publicConfig, consentEnabled, isAuthenticated, agentKeyLogin, telemetryAccountId]);

  return (
    <>
      <BuildUpdateBanner ready={ready} queryClient={queryClient} />
      {!agentKeyLogin && <ConsentBanner />}
    </>
  );
}

function Root({ ready }: { ready: boolean }) {
  const isIdentityVerifying = useAuthStore((s) => s.isIdentityVerifying);
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  // When auth resolves, redirect as needed:
  // - Authenticated user on landing → assistant
  // - Unauthenticated user on protected route → login
  useEffect(() => {
    if (!ready || isIdentityVerifying) return;
    const path = window.location.pathname;
    if (isAuthenticated && path === "/") {
      router.navigate({ to: "/assistant" });
    } else if (!isAuthenticated) {
      // Send unauthenticated users on a real protected route to login,
      // but let a genuinely-unmatched path fall through to the router's
      // 404 (`defaultNotFoundComponent`) — a mistyped URL isn't a
      // protected route, so bouncing it to login is wrong. The 404 page
      // shows a "return to main page" link for signed-out visitors.
      // `getMatchedRoutes` is a synchronous, pure matcher (no dependency
      // on the router having loaded yet), so `foundRoute === undefined`
      // is a reliable "this path matches no route" signal here.
      const pathMatchesRoute = Boolean(
        router.getMatchedRoutes(path).foundRoute,
      );
      if (pathMatchesRoute && !isPublicPath(path)) {
        preserveTelegramClaimForLogin();
        router.navigate({
          to: "/login",
          search: {
            return_to: window.location.origin + path + window.location.search,
          },
        });
      }
    }
  }, [ready, isAuthenticated, isIdentityVerifying]);

  if (!ready && !isPublicPath(window.location.pathname)) return null;
  return (
    <>
      <AccountVerificationStatus />
      <RouterProvider router={router} />
      <AccountBoundary><AccountRuntime ready={ready} /></AccountBoundary>
    </>
  );
}

function AppProviders() {
  useAuthStore((state) => state.user?.id);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    useAuthStore
      .getState()
      .checkAuth({ ephemeral: isLoginApprovalPage() })
      .finally(() => {
        setReady(true);
      });
  }, []);

  return <QueryClientProvider client={accountQueries.getClient()}><Root ready={ready} /></QueryClientProvider>;
}

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element not found");
}

// All route and runtime query observers share the current account client.
void appLanguageReady.then(() => createRoot(rootElement, crashRootOptions).render(
  <StrictMode>
    <AppProviders />
  </StrictMode>,
));

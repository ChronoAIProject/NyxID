import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ConsentBanner } from "@/components/consent-banner";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import { initTelemetry, capture, identify } from "@/lib/telemetry";
import { parsePublicPath, publishedRoute } from "./publication";
import type { publicRouter } from "./router";
export async function startPublicSession(
  router: ReturnType<typeof publicRouter>,
) {
  const consent = document.createElement("div");
  document.body.append(consent);
  createRoot(consent).render(
    <QueryClientProvider client={new QueryClient()}>
      <ConsentBanner />
    </QueryClientProvider>,
  );
  const [{ updatePublicDocsRuntime }, { PublicDocsTheme }] = await Promise.all([
    import("../docs/docs-layout-view"),
    import("./docs-theme"),
  ]);
  const updateDocs = () =>
    updatePublicDocsRuntime(
      useAuthStore.getState().isAuthenticated,
      <PublicDocsTheme />,
    );
  updateDocs();
  useAuthStore.subscribe(updateDocs);
  const redirectHome = () => {
    if (
      useAuthStore.getState().isAuthenticated &&
      ["/", "/zh"].includes(window.location.pathname)
    )
      window.location.replace("/dashboard");
  };
  void useAuthStore
    .getState()
    .checkAuth()
    .then(() => {
      initialize();
      redirectHome();
    });
  router.subscribe("onResolved", redirectHome);
  let config:
    | {
        telemetry_dsn?: string;
        telemetry_host?: string;
        telemetry_share_analytics?: boolean;
      }
    | undefined;
  const initialize = () => {
    if (config)
      initTelemetry({
        dsn: config.telemetry_dsn,
        host: config.telemetry_host,
        shareBack: config.telemetry_share_analytics === true,
        consent: useConsentStore.getState().enabled,
      });
    const user = useAuthStore.getState().user;
    if (user?.id) identify(user.id);
  };
  const state = useConsentStore.getState();
  if (!state.asked || state.enabled) {
    try {
      const response = await fetch("/api/v1/public/config");
      if (response.ok) {
        config = await response.json();
        initialize();
      }
    } catch (error) {
      console.warn(
        "[public-config] Unavailable",
        error instanceof Error ? error.name : "error",
      );
    }
  }
  useConsentStore.subscribe(initialize);
  const pageView = () => {
    const { route, locale } = parsePublicPath(window.location.pathname),
      page = publishedRoute(route);
    if (!page) return;
    capture({ name: "public_page_view", props: { page_id: page.id, locale } });
    if (route.startsWith("/solutions/"))
      capture({
        name: "solution_detail_view",
        props: { page_id: page.id, locale },
      });
  };
  document.addEventListener("click", (event) => {
    const anchor = (event.target as Element | null)?.closest("a[href]");
    const href = anchor?.getAttribute("href");
    const { route, locale } = parsePublicPath(window.location.pathname),
      page = publishedRoute(route);
    if (!href || !page) return;
    const name = href.startsWith("mailto:")
      ? "contact_mailto_open"
      : href.startsWith("https://discord.gg/")
        ? "discord_outbound"
        : href.startsWith("/docs")
          ? "docs_link_click"
          : href.startsWith("/register")
            ? "generic_signup_click"
            : null;
    if (name) capture({ name, props: { page_id: page.id, locale } });
  });
  pageView();
  router.subscribe("onResolved", pageView);
}

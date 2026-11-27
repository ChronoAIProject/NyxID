import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { websiteRelease } from "../website-a/release-config";
import { useEffect, useLayoutEffect, useState, useRef } from "react";
import type { ComponentType } from "react";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  Outlet,
  useLoaderData,
  useLocation,
  useMatches,
  useRouter,
} from "@tanstack/react-router";
import {
  PublicSearchContext,
  parsePublicSearch,
  stringifyPublicSearch,
} from "./search";
import { isAssetLoadError, retryAfterAssetError } from "@/lib/chunk-recovery";
import { DocsLayout } from "../docs/docs-layout-view";
import { WebsitePage } from "../website-a/website-page";
import { PublicDocument, parseDoc } from "./public-document";
import { parsePublicPath, publishedRoute } from "./publication";
import { docTabForSlug } from "../docs/manifest";
import { docPaths } from "./routes";
import { applyMetadata, type PublicMetadata } from "./metadata";
export interface PublicPayload {
  Page: ComponentType;
  meta: PublicMetadata;
}
export type ReadDoc = (slug: string) => Promise<string>;
export async function loadPublicPage(
  pathname: string,
  readDoc: ReadDoc,
): Promise<PublicPayload> {
  const { route, locale } = parsePublicPath(pathname),
    page = publishedRoute(route);
  if (docPaths.has(pathname)) {
    const slug = pathname.slice(6),
      markdown = slug ? await readDoc(slug) : undefined;
    const parsed = markdown
      ? parseDoc(markdown)
      : {
          title: "NyxID Documentation",
          description:
            "Broker credentials. Never expose keys. Guides for AI agents, web apps, and CLI users.",
        };
    return {
      Page: () => <PublicDocument slug={slug} markdown={markdown} />,
      meta: {
        path: pathname,
        route: pathname,
        locale: "en",
        title: slug
          ? `${parsed.title} · ${{ ai: "AI-assisted", web: "Web", cli: "CLI", shared: "Concepts" }[docTabForSlug(slug)]} Docs`
          : parsed.title,
        description: parsed.description,
        translated: false,
        indexable: true,
      },
    };
  }
  const module =
    locale === "zh"
      ? await import("../website-a/generated/zh")
      : await import("../website-a/generated/en");
  // Both home locales share the "light night" design and its interactions.
  const lightNightHome = route === "/" && Boolean(page);
  const Page = lightNightHome
    ? (await import("../website-a/nyx-home/nyx-home-page")).default
    : await module.loadPage(page ? route : "/404");
  return {
    Page,
    meta: {
      path: pathname,
      route,
      locale,
      title: lightNightHome
        ? locale === "zh" ? "能办事的 AI 助手" : "The AI assistant that gets it done"
        : (module.titles[route] ?? "Page not found"),
      description: lightNightHome
        ? locale === "zh"
          ? "NyxBot 是 NyxID 的 AI 助手。在 Telegram、X 或 NyxID 应用里提出请求，按你设定的规则订桌、回复客户、发送晨间简报。"
          : "NyxBot is the AI assistant from NyxID. Ask it on Telegram, X or the NyxID app. It books the table, replies to the customer and sends the morning brief, under rules you set."
        : route.startsWith("/solutions/") &&
            !websiteRelease.solutions[
              route.slice(11) as keyof typeof websiteRelease.solutions
            ]
          ? `${module.descriptions[route]} ${locale === "zh" ? "暂未开放。" : "Not available yet."}`
          : route === "/integrations"
            ? locale === "zh"
              ? "使用自己的账号或凭据，连接应用、AI 提供商与语音服务。"
              : "Connect apps, AI providers and voice services with your own accounts or credentials."
            : (module.descriptions[route] ?? "This page is unavailable."),
      translated: Boolean(page),
      indexable: Boolean(page?.indexable),
    },
  };
}
function Content() {
  const { Page, meta } = useLoaderData({ strict: false }) as PublicPayload;
  const location = useLocation();
  const router = useRouter();
  const navigation = useRef<{ path: string; fresh: boolean } | null>(null);
  useEffect(() => router.history.subscribe(({ location: next, action }) => {
    navigation.current = { path: next.pathname, fresh: action.type === "PUSH" || action.type === "REPLACE" };
  }), [router]);
  useEffect(() => {
    applyMetadata(meta);
    document.documentElement.dataset.publicReady = "v1";
  }, [meta]);
  useLayoutEffect(() => {
    // Wait for the destination body, rather than scrolling the old page while
    // its replacement loads. History restoration remains the router's job.
    if (meta.path !== location.pathname) return;
    const main = document.querySelector<HTMLElement>("main");
    if (!location.hash) {
      if (navigation.current?.fresh && navigation.current.path === meta.path)
        window.scrollTo({ top: 0, left: 0, behavior: "instant" });
      main?.focus({ preventScroll: true });
    }
    navigation.current = null;
  }, [meta.path, location.pathname, location.hash]);
  return <Page key={meta.path} />;
}
function PublicLayout() {
  const location = useLocation();
  const renderedPath = useMatches({ select: matches => {
    const payload = matches[matches.length - 1]?.loaderData as PublicPayload | undefined;
    return payload?.meta.path;
  } });
  return (
    <PublicSearchContext.Provider value={true}>
      {docPaths.has(renderedPath ?? location.pathname) ? (
        <Outlet />
      ) : (
        <WebsitePage renderedPath={renderedPath}>
          <Outlet />
        </WebsitePage>
      )}
    </PublicSearchContext.Provider>
  );
}
function PublicError({ error, reset }: { error: unknown; reset: () => void }) {
  useUiTranslation();
  const router = useRouter();
  const location = useLocation();
  const [retrying, setRetrying] = useState(false);
  async function retry() {
    if (isAssetLoadError(error)) {
      retryAfterAssetError();
      return;
    }
    setRetrying(true);
    try {
      await router.invalidate({ sync: true });
      reset();
    } finally {
      setRetrying(false);
    }
  }
  const body = (
    <section className="p-12" role="alert">
      <h1>{translateUi("Page unavailable")}</h1>
      <p>{translateUi("Please retry loading this page.")}</p>
      <button disabled={retrying} onClick={() => void retry()}>
        {translateUi("Retry")}
      </button>
      <a href="/">{translateUi("NyxID home")}</a>
    </section>
  );
  const tab = docTabForSlug(location.pathname.slice(6));
  return docPaths.has(location.pathname) ? (
    <DocsLayout activeTab={tab === "shared" ? "ai" : tab}>{body}</DocsLayout>
  ) : (
    body
  );
}
function PublicPending() {
  useUiTranslation();
  return <p role="status">{translateUi("Loading NyxID…")}</p>;
}
export function publicRouter(
  path?: string,
  readDoc: ReadDoc = async (slug) => {
    const response = await fetch(`/docs/${slug}.md`);
    if (!response.ok)
      throw new Error(`Documentation unavailable (${response.status})`);
    return response.text();
  },
) {
  const root = createRootRoute({ component: PublicLayout });
  const content = createRoute({
    getParentRoute: () => root,
    path: "$",
    loader: ({ location }) => loadPublicPage(location.pathname, readDoc),
    staleTime: Infinity,
    component: Content,
    pendingComponent: PublicPending,
    errorComponent: PublicError,
  });
  return createRouter({
    routeTree: root.addChildren([content]),
    history: path ? createMemoryHistory({ initialEntries: [path] }) : undefined,
    parseSearch: parsePublicSearch,
    stringifySearch: stringifyPublicSearch,
    defaultPreload: false,
    defaultPendingMs: 10000,
    // SSG must not emit TanStack's self-removing inline restoration script.
    // The browser router still restores client-side history after hydration.
    scrollRestoration: !path,
    scrollRestorationBehavior: "instant",
  });
}

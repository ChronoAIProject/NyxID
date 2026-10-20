import { useEffect, useState, useRef } from "react";
import type { ComponentType } from "react";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  Outlet,
  useLoaderData,
  useLocation,
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
  const Page = await module.loadPage(page ? route : "/404");
  return {
    Page,
    meta: {
      path: pathname,
      route,
      locale,
      title: module.titles[route] ?? "Page not found",
      description: route.startsWith("/solutions/")
        ? `${module.descriptions[route]} ${locale === "zh" ? "暂未开放。" : "Not available yet."}`
        : route === "/integrations"
          ? locale === "zh"
            ? "浏览 NyxID 集成，区分已上线和即将上线的服务；各项可用状态以页面标记为准。"
            : "Browse NyxID integrations, including live and coming soon services. Check the status shown for each integration."
          : (module.descriptions[route] ?? "This page is unavailable."),
      translated: Boolean(page),
      indexable: Boolean(page?.indexable),
    },
  };
}
function Content() {
  const { Page, meta } = useLoaderData({ strict: false }) as PublicPayload;
  const location = useLocation();
  useEffect(() => {
    applyMetadata(meta);
    document.documentElement.dataset.publicReady = "v1";
  }, [meta]);
  useEffect(() => {
    const main = document.querySelector<HTMLElement>("main");
    if (location.hash) {
      try {
        document
          .getElementById(decodeURIComponent(location.hash))
          ?.scrollIntoView();
      } catch {
        /* Malformed anchors leave the readable body in place. */
      }
    } else {
      main?.focus({ preventScroll: true });
    }
  }, [location.pathname, location.hash]);
  return <Page />;
}
function PublicLayout() {
  const location = useLocation();
  const router = useRouter();
  const [navigationRevision, setNavigationRevision] = useState(0);
  const lastSuccessfulHref = useRef(location.href);
  useEffect(
    () =>
      router.subscribe("onResolved", ({ toLocation }) => {
        if (
          router.state.matches.every((match) => match.status === "success") &&
          toLocation.href !== lastSuccessfulHref.current
        ) {
          lastSuccessfulHref.current = toLocation.href;
          setNavigationRevision((value) => value + 1);
        }
      }),
    [router],
  );
  return (
    <PublicSearchContext.Provider value={true}>
      {docPaths.has(location.pathname) ? (
        <Outlet />
      ) : (
        <WebsitePage navigationRevision={navigationRevision}>
          <Outlet />
        </WebsitePage>
      )}
    </PublicSearchContext.Provider>
  );
}
function PublicError({ error, reset }: { error: unknown; reset: () => void }) {
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
      <h1>Page unavailable</h1>
      <p>Please retry loading this page.</p>
      <button disabled={retrying} onClick={() => void retry()}>
        Retry
      </button>
      <a href="/">NyxID home</a>
    </section>
  );
  const tab = docTabForSlug(location.pathname.slice(6));
  return docPaths.has(location.pathname) ? (
    <DocsLayout activeTab={tab === "shared" ? "ai" : tab}>{body}</DocsLayout>
  ) : (
    body
  );
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
    pendingComponent: () => <p role="status">Loading NyxID…</p>,
    errorComponent: PublicError,
  });
  return createRouter({
    routeTree: root.addChildren([content]),
    history: path ? createMemoryHistory({ initialEntries: [path] }) : undefined,
    parseSearch: parsePublicSearch,
    stringifySearch: stringifyPublicSearch,
    defaultPreload: false,
    defaultPendingMs: 10000,
    scrollRestoration: true,
  });
}

import { websiteVisibility, websiteConfiguration } from "./release-config";
import { useBrowserContext } from "../public-site/browser-context";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { ReactNode, SyntheticEvent } from "react";
import { useLocation } from "@tanstack/react-router";
import { usePublicThemeElement } from "../public-site/public-theme";
import { PublicFooter, PublicHeader } from "../public-site/public-chrome";
import {
  parsePublicPath,
  publicationPath,
  publishedRoute,
} from "../public-site/publication";
import { WebsiteContext } from "./website-context";
import type { WebsiteState } from "./website-context";
import {
  authHref,
  siteHref,
  legacyDestination,
  injectRedditDefaultCode,
} from "./auth-links";

import integrations from "./generated/integrations.json";
import "./website-a.css";
import "./visuals/visuals.css";
import { useVisualMotion } from "./visuals/motion";
import { VisualMotionProvider } from "./visuals/motion-provider";

const English = lazy(() => import("./generated/en"));
const Chinese = lazy(() => import("./generated/zh"));
// The English home page uses the "light night" design, with its own menu and footer.
const NyxHomePage = lazy(() => import("./nyx-home/nyx-home-page"));

function Website({
  children,
  navigationRevision,
}: {
  children?: ReactNode;
  navigationRevision?: number;
}) {
  const { mode } = useVisualMotion();
  const publicRef = useRef<HTMLDivElement>(null);
  usePublicThemeElement(publicRef);
  const location = useLocation();
  // Canonical HTML has no visitor query. The first client tree must match it;
  // apply browser context after hydration and after subsequent URL changes.
  const urlContext = useBrowserContext();
  useEffect(() => {
    injectRedditDefaultCode();
    window.dispatchEvent(new Event("nyxid:public-url-context"));
  }, [location.pathname, location.searchStr, location.hash]);
  const { locale: language, route } = parsePublicPath(location.pathname);
  const [menu, setMenu] = useState({ revision: navigationRevision, open: false });
  const menuOpen = menu.revision === navigationRevision && menu.open;
  const [filters, setFilters] = useState<Record<string, string>>({
    catF: "all",
    chF: "",
    blogF: "all",
    intAv: "",
  });
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("");
  const [demoIndex, setDemoIndex] = useState(0);
  const [demoStopped, setDemoStopped] = useState(false);

  useEffect(() => {
    const previous = document.documentElement.lang;
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";

    return () => {
      document.documentElement.lang = previous;
    };
  }, [language, location.pathname]);

  useEffect(() => {
    if (demoStopped || route !== "/nyxbot") return;
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const timer = window.setInterval(() => {
      if (!media.matches && !document.hidden)
        setDemoIndex((index) => (index + 1) % 4);
    }, 7000);
    return () => window.clearInterval(timer);
  }, [demoStopped, route]);

  // Migrate old prototype links without installing any mock auth/start handlers.
  useEffect(() => {
    const destination = legacyDestination(window.location.hash);
    if (destination) window.location.replace(destination);
  }, []);

  const isFilterSelected = (group: string, value: string) =>
    filters[group] === value;
  const integrationVisible = (name: string, cat: string, available: string) =>
    (!query.trim() || name.includes(query.trim().toLowerCase())) &&
    (!category || cat === category) &&
    (!filters.intAv || available === filters.intAv);
  const integrationCount = integrations.filter((item) =>
    integrationVisible(item.name, item.cat, item.available),
  ).length;
  const state: WebsiteState = {
    language,
    route,
    menuOpen,
    localeHref: (next) => {
      const params = new URLSearchParams(urlContext.search);
      params.delete("lang");
      const query = params.toString();
      return (
        publicationPath(route, next) +
        (query ? `?${query}` : "") +
        urlContext.hash
      );
    },
    setLanguage: (next) => {
      window.location.assign(publicationPath(route, next) + urlContext.search);
    },
    toggleMenu: () => setMenu({ revision: navigationRevision, open: !menuOpen }),
    authHref: (kind) => authHref(kind, urlContext.search),
    siteHref: (path) => {
      const url = new URL(path, "https://nyx.chrono-ai.fun");
      const localized = publishedRoute(url.pathname)
        ? publicationPath(url.pathname, language)
        : url.pathname;
      const params = new URLSearchParams(urlContext.search);
      params.delete("lang");
      return siteHref(localized + url.search + url.hash, `?${params}`);
    },
    imageFallback: (event: SyntheticEvent<HTMLImageElement>) => {
      // Hide a failed icon while its adjacent provider name remains readable.
      event.currentTarget.style.visibility = "hidden";
    },
    filterClass: (group, value) =>
      `pill ${isFilterSelected(group, value) ? "on" : ""}`,
    setFilter: (group, value) =>
      setFilters((current) => ({
        ...current,
        [group]: group === "chF" && current[group] === value ? "" : value,
      })),
    isFilterSelected,
    solutionVisible: (cat, channels) =>
      (filters.catF === "all" || cat === filters.catF) &&
      (!filters.chF || channels.split(",").includes(filters.chF)),
    postVisible: (cat) => filters.blogF === "all" || cat === filters.blogF,
    query,
    category,
    setQuery,
    setCategory,
    integrationVisible,
    integrationGroupCount: (available) =>
      integrations.filter(
        (item) =>
          item.available === available &&
          integrationVisible(item.name, item.cat, item.available),
      ).length,
    integrationCountLabel:
      language === "zh"
        ? `显示 ${integrationCount} / ${integrations.length}`
        : `Showing ${integrationCount} of ${integrations.length}`,
    demoIndex,
    selectDemo: (index) => {
      setDemoStopped(true);
      setDemoIndex(index);
    },
    stopDemo: () => setDemoStopped(true),
  };
  // The English home page brings its own menu and footer, so it skips this frame.
  // The public site passes it in as children (its router loads it); the app renders it here.
  if (language === "en" && route === "/") {
    return (
      <WebsiteContext.Provider value={state}>
        <Suspense fallback={null}>{children ?? <NyxHomePage />}</Suspense>
      </WebsiteContext.Provider>
    );
  }
  const Content = language === "zh" ? Chinese : English;
  return (
    <WebsiteContext.Provider value={state}>
      <div
        ref={publicRef}
        className="website-a nyx-public"
        data-website-visibility={JSON.stringify(websiteVisibility)}
        data-website-config={encodeURIComponent(websiteConfiguration)}
        data-visual-version="4.1"
        data-visual-route={state.route}
        data-visual-motion={mode}
        lang={language === "zh" ? "zh-CN" : "en"}
      >
        <a href="#website-main" className="website-skip">
          {language === "zh" ? "跳到正文" : "Skip to content"}
        </a>
        <PublicHeader language={language} route={route} siteHref={state.siteHref} authHref={state.authHref} localeHref={state.localeHref} />
        <main id="website-main" tabIndex={-1}>
          <Suspense
            fallback={
              <div className="wrap" role="status" style={{ padding: 48 }}>
                {language === "zh" ? "加载中…" : "Loading…"}
              </div>
            }
          >
            {children ?? <Content />}
          </Suspense>
        </main>
        <PublicFooter language={language} siteHref={state.siteHref} localeHref={state.localeHref} />
      </div>
    </WebsiteContext.Provider>
  );
}

export function WebsitePage({
  children,
  navigationRevision,
}: {
  children?: ReactNode;
  navigationRevision?: number;
}) {
  return (
    <VisualMotionProvider>
      <Website navigationRevision={navigationRevision}>{children}</Website>
    </VisualMotionProvider>
  );
}

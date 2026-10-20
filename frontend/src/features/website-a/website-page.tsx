import { useBrowserContext } from "../public-site/browser-context";
import { lazy, Suspense, useEffect, useState } from "react";
import type { ReactNode, SyntheticEvent } from "react";
import { useLocation } from "@tanstack/react-router";
import { WEBSITE_THEME } from "./website-theme";
import {
  parsePublicPath,
  publicationPath,
  publishedRoute,
} from "../public-site/publication";
import EnHeader from "./generated/en/WebsiteHeader";
import EnFooter from "./generated/en/WebsiteFooter";
import ZhHeader from "./generated/zh/WebsiteHeader";
import ZhFooter from "./generated/zh/WebsiteFooter";
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

function Website({ children }: { children?: ReactNode }) {
  const { mode } = useVisualMotion();
  const location = useLocation();
  // Canonical HTML has no visitor query. The first client tree must match it;
  // apply browser context after hydration and after subsequent URL changes.
  const urlContext = useBrowserContext();
  useEffect(() => {
    injectRedditDefaultCode();
    window.dispatchEvent(new Event("nyxid:public-url-context"));
  }, [location.pathname, location.searchStr, location.hash]);
  const { locale: language, route } = parsePublicPath(location.pathname);
  const [menuOpen, setMenuOpen] = useState(false);
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
    toggleMenu: () => setMenuOpen((open) => !open),
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
  const Content = language === "zh" ? Chinese : English;
  const Header = language === "zh" ? ZhHeader : EnHeader;
  const Footer = language === "zh" ? ZhFooter : EnFooter;
  return (
    <WebsiteContext.Provider value={state}>
      <div
        className="website-a"
        data-theme={WEBSITE_THEME}
        data-visual-version="4.1"
        data-visual-route={state.route}
        data-visual-motion={mode}
        lang={language === "zh" ? "zh-CN" : "en"}
      >
        <a href="#website-main" className="website-skip">
          {language === "zh" ? "跳到正文" : "Skip to content"}
        </a>
        <nav className={"nav " + (menuOpen ? "open" : "")} id="nav">
          <Header />
        </nav>
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
        <footer id="foot">
          <Footer />
        </footer>
      </div>
    </WebsiteContext.Provider>
  );
}

export function WebsitePage({ children }: { children?: ReactNode }) {
  return (
    <VisualMotionProvider>
      <Website>{children}</Website>
    </VisualMotionProvider>
  );
}

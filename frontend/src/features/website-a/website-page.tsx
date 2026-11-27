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

import "./website-a.css";

const English = lazy(() => import("./generated/en"));
const Chinese = lazy(() => import("./generated/zh"));
// Both homepage locales use the "light night" design, with its own menu and footer.
const NyxHomePage = lazy(() => import("./nyx-home/nyx-home-page"));

function Website({
  children,
  renderedPath,
}: {
  children?: ReactNode;
  renderedPath?: string;
}) {
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
  // Keep the current page's frame while the next route's loader is pending.
  // Navigation links still follow the requested URL and its locale.
  const rendered = parsePublicPath(renderedPath ?? location.pathname);
  const [filters, setFilters] = useState<Record<string, string>>({
    catF: "all",
    chF: "",
  });

  useEffect(() => {
    const previous = document.documentElement.lang;
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";

    return () => {
      document.documentElement.lang = previous;
    };
  }, [language, location.pathname]);

  // Migrate old prototype links without installing any mock auth/start handlers.
  useEffect(() => {
    const destination = legacyDestination(window.location.hash);
    if (destination) window.location.replace(destination);
  }, []);

  const isFilterSelected = (group: string, value: string) =>
    filters[group] === value;
  const state: WebsiteState = {
    language,
    route,
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
  };
  // The homepage brings its own menu and footer in both locales, so it skips this frame.
  // The public site passes it in as children (its router loads it); the app renders it here.
  if (rendered.route === "/") {
    return (
      <WebsiteContext.Provider value={state}>
        <Suspense fallback={null}>{children ?? <NyxHomePage key={language} />}</Suspense>
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
  renderedPath,
}: {
  children?: ReactNode;
  renderedPath?: string;
}) {
  return <Website renderedPath={renderedPath}>{children}</Website>;
}

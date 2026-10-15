import { lazy, Suspense, useEffect, useState } from "react";
import type { SyntheticEvent } from "react";
import { useLocation } from "@tanstack/react-router";
import { I18nextProvider, useTranslation } from "react-i18next";
import { WebsiteContext } from "./website-context";
import type { Language, WebsiteState } from "./website-context";
import {
  authHref,
  siteHref,
  legacyDestination,
  injectRedditDefaultCode,
} from "./auth-links";
import { websiteI18n } from "./i18n";
import integrations from "./generated/integrations.json";
import "./website-a.css";

const English = lazy(() => import("./generated/en"));
const Chinese = lazy(() => import("./generated/zh"));

function Website() {
  const location = useLocation();
  const { i18n } = useTranslation(undefined, { i18n: websiteI18n });
  const language: Language = i18n.language === "zh" ? "zh" : "en";
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
  injectRedditDefaultCode();

  useEffect(() => {
    const previous = document.documentElement.lang;
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";

    return () => {
      document.documentElement.lang = previous;
    };
  }, [language, location.pathname]);

  useEffect(() => {
    if (demoStopped || location.pathname !== "/nyxbot") return;
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const timer = window.setInterval(() => {
      if (!media.matches && !document.hidden)
        setDemoIndex((index) => (index + 1) % 4);
    }, 7000);
    return () => window.clearInterval(timer);
  }, [demoStopped, location.pathname]);

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
    route: location.pathname.replace(/\/$/, "") || "/",
    menuOpen,
    setLanguage: (next) => {
      void i18n.changeLanguage(next);
      try {
        localStorage.setItem("nyx-lang", next);
      } catch {
        /* In-memory language remains available. */
      }
      // Explicit URL language must agree with the newly selected language on reload.
      const url = new URL(window.location.href);
      if (url.searchParams.has("lang")) {
        url.searchParams.set("lang", next);
        window.history.replaceState(window.history.state, "", url);
      }
    },
    toggleMenu: () => setMenuOpen((open) => !open),
    authHref,
    siteHref,
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
  return (
    <WebsiteContext.Provider value={state}>
      <div
        className="website-a"
        data-theme="dark"
        lang={language === "zh" ? "zh-CN" : "en"}
      >
        <a href="#website-main" className="website-skip">
          {language === "zh" ? "跳到正文" : "Skip to content"}
        </a>
        <Suspense
          fallback={
            <div className="wrap" role="status" style={{ padding: 48 }}>
              {language === "zh" ? "加载中…" : "Loading…"}
            </div>
          }
        >
          <Content key={`${location.pathname}-${language}`} />
        </Suspense>
      </div>
    </WebsiteContext.Provider>
  );
}

export function WebsitePage() {
  return (
    <I18nextProvider i18n={websiteI18n}>
      <Website />
    </I18nextProvider>
  );
}

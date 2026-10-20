import { createContext, useContext } from "react";
import type { SyntheticEvent } from "react";

export type Language = "en" | "zh";
export interface WebsiteState {
  language: Language;
  route: string;
  menuOpen: boolean;
  setLanguage: (language: Language) => void;
  toggleMenu: () => void;
  localeHref: (language: Language) => string;
  authHref: (kind: "login" | "register") => string;
  siteHref: (pathname: string) => string;
  imageFallback: (event: SyntheticEvent<HTMLImageElement>) => void;
  filterClass: (group: string, value: string) => string;
  setFilter: (group: string, value: string) => void;
  isFilterSelected: (group: string, value: string) => boolean;
  solutionVisible: (category: string, channels: string) => boolean;
  postVisible: (category: string) => boolean;
  query: string;
  category: string;
  setQuery: (query: string) => void;
  setCategory: (category: string) => void;
  integrationVisible: (
    name: string,
    category: string,
    available: string,
  ) => boolean;
  integrationGroupCount: (available: string) => number;
  integrationCountLabel: string;
  demoIndex: number;
  selectDemo: (index: number) => void;
  stopDemo: () => void;
}

export const WebsiteContext = createContext<WebsiteState | null>(null);
export function useWebsite() {
  const state = useContext(WebsiteContext);
  if (!state) throw new Error("Website components require WebsiteContext");
  return state;
}

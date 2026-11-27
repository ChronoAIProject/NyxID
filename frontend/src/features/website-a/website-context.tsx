import { createContext, useContext } from "react";
import type { SyntheticEvent } from "react";

export type Language = "en" | "zh";
export interface WebsiteState {
  language: Language;
  route: string;
  localeHref: (language: Language) => string;
  authHref: (kind: "login" | "register") => string;
  siteHref: (pathname: string) => string;
  imageFallback: (event: SyntheticEvent<HTMLImageElement>) => void;
  filterClass: (group: string, value: string) => string;
  setFilter: (group: string, value: string) => void;
  isFilterSelected: (group: string, value: string) => boolean;
  solutionVisible: (category: string, channels: string) => boolean;
}

export const WebsiteContext = createContext<WebsiteState | null>(null);
export function useWebsite() {
  const state = useContext(WebsiteContext);
  if (!state) throw new Error("Website components require WebsiteContext");
  return state;
}

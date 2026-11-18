import { drawerSectionInfo } from "./drawer-section-info";
import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useRef,
  useId,
} from "react";
import type { AgentDetailsTab } from "./assistant/search";

export type DrawerSearchTab = AgentDetailsTab;
export interface DrawerSearchMetadata {
  readonly id: string;
  readonly title: string;
  readonly breadcrumb?: string;
  readonly keywords?: string;
}
export interface DrawerSearchEntry extends DrawerSearchMetadata {
  readonly tab: DrawerSearchTab;
  readonly pages?: readonly string[];
  readonly reveal?: () => void;
  readonly target?: () => HTMLElement | null;
}
export interface DrawerSearchPage {
  readonly id: string;
  readonly tab: DrawerSearchTab;
  readonly open: boolean;
  readonly level?: number;
  readonly leave: () => Promise<boolean>;
}
export const DRAWER_SEARCH_TABS: Record<DrawerSearchTab, string> = {
  overview: "Overview",
  grants: "Access",
  skills: "Skills",
  memory: "Memory",
  channels: "Channels",
};
export function normalizeDrawerSearch(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}
export function matchDrawerEntries(
  entries: readonly DrawerSearchEntry[],
  query: string,
  limit = 80,
) {
  const normalized = normalizeDrawerSearch(query).trim();
  if (!normalized) return [];
  const words = normalized.split(/\s+/);
  return entries
    .flatMap((entry) => {
      const title = normalizeDrawerSearch(entry.title);
      const all = normalizeDrawerSearch(
        `${entry.title} ${entry.breadcrumb ?? DRAWER_SEARCH_TABS[entry.tab]} ${entry.keywords ?? ""}`,
      );
      if (!words.every((word) => all.includes(word))) return [];
      const rank =
        title === normalized
          ? 0
          : title.startsWith(normalized)
            ? 1
            : words.every((word) => title.includes(word))
              ? 2
              : 3;
      return [{ entry, rank }];
    })
    .sort(
      (a, b) =>
        a.rank - b.rank ||
        a.entry.title.localeCompare(b.entry.title) ||
        a.entry.id.localeCompare(b.entry.id),
    )
    .slice(0, limit)
    .map(({ entry }) => entry);
}
/** Offsets refer to the original text, including combining marks and surrogate pairs. */
export function drawerMatchRanges(
  text: string,
  query: string,
): [number, number][] {
  const offsets: [number, number][] = [];
  let normalized = "",
    offset = 0;
  for (const character of text) {
    const next = normalizeDrawerSearch(character);
    if (!next && offsets.length) offsets.at(-1)![1] = offset + character.length;
    for (let index = 0; index < next.length; index++)
      offsets.push([offset, offset + character.length]);
    normalized += next;
    offset += character.length;
  }
  const ranges: [number, number][] = [];
  for (const word of normalizeDrawerSearch(query)
    .trim()
    .split(/\s+/)
    .filter(Boolean)) {
    let index = normalized.indexOf(word);
    while (index !== -1) {
      ranges.push([offsets[index]![0], offsets[index + word.length - 1]![1]]);
      index = normalized.indexOf(word, index + word.length);
    }
  }
  return ranges
    .sort((a, b) => a[0] - b[0])
    .reduce<[number, number][]>((merged, range) => {
      const last = merged.at(-1);
      if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1]);
      else merged.push([...range]);
      return merged;
    }, []);
}
export const DrawerSearchContext = createContext<{
  readonly register: (
    owner: string,
    entries: readonly DrawerSearchEntry[],
  ) => void;
  readonly remove: (owner: string) => void;
  readonly registerPage: (owner: string, page: DrawerSearchPage | null) => void;
  readonly searching: boolean;
} | null>(null);
export const DrawerSearchRevealContext = createContext<{ readonly pages: readonly string[]; readonly reveal?: () => void } | null>(null);
export const DrawerSearchTabContext = createContext<DrawerSearchTab | null>(
  null,
);

/** Registration follows section data and is a no-op outside an agent drawer. */
export function useDrawerSearchEntries(
  entries: readonly Omit<DrawerSearchEntry, "tab">[],
) {
  const context = useContext(DrawerSearchContext);
  const tab = useContext(DrawerSearchTabContext);
  const inherited = useContext(DrawerSearchRevealContext);
  const currentEntries = entries.map((entry) => ({ ...inherited, ...entry, keywords: `${entry.keywords ?? ""} ${[entry.title, ...(entry.breadcrumb?.split(" › ") ?? [])].map((title) => drawerSectionInfo(title) ?? "").join(" ")}` }));
  const id = useId();
  const latest = useRef(currentEntries);
  useLayoutEffect(() => {
    latest.current = currentEntries;
  });
  const signature = JSON.stringify(
    currentEntries.map(({ id, title, breadcrumb, keywords, pages }) => ({
      id,
      title,
      breadcrumb,
      keywords,
      pages,
    })),
  );
  const register = context?.register,
    remove = context?.remove;
  useLayoutEffect(() => {
    if (tab)
      register?.(
        id,
        latest.current.map((entry) => ({
          ...entry,
          tab,
          reveal: () =>
            latest.current
              .find((current) => current.id === entry.id)
              ?.reveal?.(),
          target: () =>
            latest.current
              .find((current) => current.id === entry.id)
              ?.target?.() ?? null,
        })),
      );
    return () => remove?.(id);
  }, [register, remove, id, tab, signature]);
}
export function useDrawerSearchTarget(search?: DrawerSearchMetadata) {
  const target = useRef<HTMLElement | null>(null);
  useDrawerSearchEntries(
    search ? [{ ...search, target: () => target.current }] : [],
  );
  return useCallback((element: HTMLElement | null) => {
    target.current = element;
  }, []);
}
export function useDrawerSearchPage(
  page: Omit<DrawerSearchPage, "tab"> | null,
) {
  const register = useContext(DrawerSearchContext)?.registerPage;
  const tab = useContext(DrawerSearchTabContext);
  const owner = useId();
  const latest = useRef(page);
  useLayoutEffect(() => {
    latest.current = page;
  });
  const id = page?.id,
    open = page?.open,
    level = page?.level;
  useLayoutEffect(() => {
    if (tab && id && open !== undefined)
      register?.(owner, {
        id,
        tab,
        open,
        level,
        leave: () => latest.current?.leave() ?? Promise.resolve(true),
      });
    return () => register?.(owner, null);
  }, [register, tab, owner, id, open, level]);
}
export function useDrawerSearching() {
  return useContext(DrawerSearchContext)?.searching ?? false;
}

export function useDrawerSearchEnabled() {
  return useContext(DrawerSearchContext) !== null;
}

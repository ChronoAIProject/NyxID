import { CompactScroll } from "@/components/ui/compact-title";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  createContext,
  useContext,
  type RefObject,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { Search, X } from "lucide-react";
import {
  DrawerSearchContext,
  DRAWER_SEARCH_TABS,
  drawerMatchRanges,
  matchDrawerEntries,
  type DrawerSearchEntry,
  type DrawerSearchPage,
  type DrawerSearchTab,
} from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
} from "@/components/ui/grouped-list";
import { Button } from "@/components/ui/button";

export function DrawerSearchProvider({
  children,
  query,
}: {
  readonly children: ReactNode;
  readonly query: string;
}) {
  const [entries, setEntries] = useState<
    ReadonlyMap<string, readonly DrawerSearchEntry[]>
  >(new Map());
  const pages = useRef(new Map<string, DrawerSearchPage>());
  const register = useCallback(
    (id: string, value: readonly DrawerSearchEntry[]) =>
      setEntries((current) => new Map(current).set(id, value)),
    [],
  );
  const remove = useCallback(
    (id: string) =>
      setEntries((current) => {
        const next = new Map(current);
        next.delete(id);
        return next;
      }),
    [],
  );
  const registerPage = useCallback(
    (id: string, page: DrawerSearchPage | null) => {
      if (page) pages.current.set(id, page);
      else pages.current.delete(id);
    },
    [],
  );
  return (
    <DrawerSearchContext.Provider
      value={{
        register,
        remove,
        registerPage,
        searching: Boolean(query.trim()),
      }}
    >
      <DrawerSearchResultsContext.Provider
        value={{ entries: [...entries.values()].flat(), pages }}
      >
        {children}
      </DrawerSearchResultsContext.Provider>
    </DrawerSearchContext.Provider>
  );
}
const DrawerSearchResultsContext = createContext<{
  entries: DrawerSearchEntry[];
  pages: RefObject<Map<string, DrawerSearchPage>>;
} | null>(null);
function Match({
  text,
  query,
}: {
  readonly text: string;
  readonly query: string;
}) {
  let position = 0;
  const parts: ReactNode[] = [];
  for (const [start, end] of drawerMatchRanges(text, query)) {
    parts.push(
      text.slice(position, start),
      <mark
        key={start}
        className="bg-transparent font-semibold text-primary-text"
      >
        {text.slice(start, end)}
      </mark>,
    );
    position = end;
  }
  parts.push(text.slice(position));
  return <>{parts}</>;
}
export function AgentDrawerSearch({
  host,
  open,
  setOpen,
  title,
  query,
  setQuery,
  selected,
  selectTab,
  container,
}: {
  readonly host: HTMLElement | null;
  readonly open: boolean;
  readonly setOpen: (open: boolean) => void;
  readonly title: string;
  readonly query: string;
  readonly setQuery: (value: string) => void;
  readonly selected: DrawerSearchTab;
  readonly selectTab: (tab: DrawerSearchTab) => void;
  readonly container: HTMLElement | null;
}) {
  const context = useContext(DrawerSearchResultsContext)!;
  const searchLabel = `Search ${title} agent details`;
  const input = useRef<HTMLInputElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const [placeholder, setPlaceholder] = useState(searchLabel);
  useEffect(() => { if (open) input.current?.focus({ preventScroll: true }); }, [open]);
  useLayoutEffect(() => {
    const node = input.current;
    if (!node || !open) return;
    const measure = () => {
      if (!node.clientWidth) return;
      const context = document.createElement("canvas").getContext("2d");
      if (!context) return;
      const style = getComputedStyle(node);
      context.font = style.font;
      const width = node.clientWidth - parseFloat(style.paddingLeft || "0") - parseFloat(style.paddingRight || "0");
      setPlaceholder([searchLabel, `Search ${title}`, "Search agent"].find((text) => context.measureText(text).width <= width) ?? "Search");
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [open, searchLabel, title, host]);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const [pending, setPending] = useState<DrawerSearchEntry>();
  const [highlighted, setHighlighted] = useState<HTMLElement>();
  const [opening, setOpening] = useState(false);
  const searching = Boolean(query.trim());
  const matches = matchDrawerEntries(context.entries, query);
  const groups = Object.entries(DRAWER_SEARCH_TABS).flatMap(([tab, label]) => {
    const rows = matches.filter((entry) => entry.tab === tab);
    return rows.length ? [{ label, rows }] : [];
  });
  const ordered = groups.flatMap((group) => group.rows);
  useEffect(() => {
    if (!container) return;
    function keydown(event: KeyboardEvent) {
      if (event.defaultPrevented || !container?.contains(event.target as Node))
        return;
      if (event.key === "Escape" && searching) {
        event.preventDefault();
        event.stopPropagation();
        setQuery("");
        input.current?.focus();
        return;
      }
      const element = event.target as HTMLElement;
      const editing = Boolean(
        element.closest(
          "input,textarea,[contenteditable=true],[role=combobox]",
        ),
      );
      if (
        !editing &&
        (event.key === "/" ||
          ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f"))
      ) {
        event.preventDefault();
        event.stopPropagation();
        setOpen(true);
        input.current?.focus();
      }
    }
    container.addEventListener("keydown", keydown, true);
    return () => container.removeEventListener("keydown", keydown, true);
  }, [container, searching, setQuery, setOpen]);
  useEffect(() => {
    if (!pending || !container || searching) return;
    let frame = 0, attempts = 0;
    const focus = () => {
      const target =
        pending.target?.() ??
        [
          ...container.querySelectorAll<HTMLElement>(
            "[data-drawer-search-target]",
          ),
        ].find((element) => element.dataset.drawerSearchTarget === pending.id);
      if (target?.isConnected && !target.closest("[hidden],[inert]")) {
        target.scrollIntoView?.({ block: "center", behavior: "instant" });
        if (!target.hasAttribute("tabindex")) target.tabIndex = -1;
        target.focus({ preventScroll: true });
        setHighlighted(target);
        setPending(undefined);
      } else if (++attempts < 120) frame = requestAnimationFrame(focus);
      else {
        input.current?.focus();
        setPending(undefined);
      }
    };
    frame = requestAnimationFrame(focus);
    return () => {
      cancelAnimationFrame(frame);

    };
  }, [pending, container, searching]);
  useEffect(() => {
    if (!highlighted) return;
    highlighted.setAttribute("data-search-highlight", "");
    const timer = setTimeout(() => setHighlighted(undefined), 1600);
    return () => { clearTimeout(timer); highlighted.removeAttribute("data-search-highlight"); };
  }, [highlighted]);
  async function reveal(entry: DrawerSearchEntry) {
    if (opening) return;
    setOpening(true);
    try {
      for (const page of [...context.pages.current.values()].sort(
        (a, b) => (b.level ?? 1) - (a.level ?? 1),
      )) {
        if (
          page.open &&
          (page.tab === selected || page.tab === entry.tab) &&
          !entry.pages?.includes(page.id) &&
          !(await page.leave())
        )
          return;
      }
      selectTab(entry.tab);
      entry.reveal?.();
      setPending(entry);
      setQuery("");
    } finally {
      setOpening(false);
    }
  }
  function move(event: React.KeyboardEvent, id?: string) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    if (!ordered.length) return;
    event.preventDefault();
    const index = ordered.findIndex((entry) => entry.id === id);
    const next =
      index < 0
        ? event.key === "ArrowDown"
          ? 0
          : ordered.length - 1
        : (index + (event.key === "ArrowDown" ? 1 : -1) + ordered.length) %
          ordered.length;
    buttons.current.get(ordered[next]!.id)?.focus();
  }
  function closeSearch() {
    setQuery("");
    setOpen(false);
    requestAnimationFrame(() => toggle.current?.focus({ preventScroll: true }));
  }
  return (
    <>
      {host && createPortal(
        <div data-drawer-search="" data-open={open} className="agent-header-search" role="search" aria-label={searchLabel}>
          <button ref={toggle} data-search-toggle data-icon-button="" type="button" aria-label={searchLabel} aria-expanded={open} className="agent-header-icon" hidden={open} onClick={() => setOpen(true)}>
            <Search aria-hidden className="size-4" />
          </button>
          <div className="agent-header-search-field flex min-h-8 items-center gap-1 rounded-full bg-overlay-strong px-2" inert={!open} aria-hidden={!open || undefined}>
            <Search aria-hidden className="agent-header-search-glyph size-3 shrink-0 text-text-tertiary" />
            <input
              ref={input}
              type="search"
              role="searchbox"
              aria-label={searchLabel}
              placeholder={placeholder}
              maxLength={200}
              value={query}
              tabIndex={open ? 0 : -1}
              onChange={(event) => setQuery(event.target.value)}
              onBlur={(event) => {
                if (!query.trim() && !event.currentTarget.parentElement?.contains(event.relatedTarget as Node)) setOpen(false);
              }}
              onKeyDown={(event) => {
                move(event);
                if (event.key === "Enter" && ordered[0]) {
                  event.preventDefault();
                  void reveal(ordered[0]);
                }
                if (event.key === "Escape" && !searching) {
                  event.preventDefault();
                  event.stopPropagation();
                  closeSearch();
                }
              }}
              className="min-h-8 min-w-0 flex-1 border-0 bg-transparent py-1 type-body shadow-none outline-none placeholder:text-text-tertiary [&::-webkit-search-cancel-button]:appearance-none"
            />
            {query && <Button type="button" variant="ghost" size="icon" aria-label="Clear agent search" className="size-6" onClick={() => { setQuery(""); input.current?.focus(); }}><X aria-hidden className="size-3" /></Button>}
          </div>
        </div>, host)}
      <p role="status" aria-live="polite" aria-atomic="true" className="sr-only">
        {searching ? `${matches.length}${matches.length === 80 ? " or more" : ""} ${matches.length === 1 ? "result" : "results"} for ${query}` : ""}
      </p>
      {searching && (
        <CompactScroll level={100}
          data-drawer-search-results=""
          aria-label="Agent search results"
          className="assistant-scrollbar order-2 min-h-0 flex-1 space-y-6 overflow-y-auto px-4 pb-4"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              setQuery("");
              input.current?.focus();
            }
          }}
        >
          {!matches.length ? (
            <p className="px-4 py-6 text-14 text-text-tertiary">
              No results for “{query}”.
            </p>
          ) : (
            groups.map(({ label, rows }) => (
              <GroupedList key={label} title={label}>
                <ul aria-label={`${label} search results`}>
                  {rows.map((entry) => (
                    <GroupedRow
                      key={entry.id}
                      asChild
                      interactive
                      className="!p-0"
                    >
                      <li>
                        <button
                          type="button"
                          className="flex min-h-11 w-full px-4 py-2.5 text-left focus-visible:outline-none"
                          aria-label={`${entry.title} ${entry.breadcrumb ?? DRAWER_SEARCH_TABS[entry.tab]}`}
                          ref={(element) => {
                            if (element) buttons.current.set(entry.id, element);
                            else buttons.current.delete(entry.id);
                          }}
                          disabled={opening}
                          onKeyDown={(event) => move(event, entry.id)}
                          onClick={() => void reveal(entry)}
                        >
                          <span className="min-w-0 flex-1 break-words">
                            <span className="block">
                              <Match text={entry.title} query={query} />
                            </span>
                            <span className="block type-secondary text-text-tertiary">
                              <Match
                                text={
                                  entry.breadcrumb ??
                                  DRAWER_SEARCH_TABS[entry.tab]
                                }
                                query={query}
                              />
                            </span>
                          </span>
                        </button>
                      </li>
                    </GroupedRow>
                  ))}
                </ul>
              </GroupedList>
            ))
          )}
        </CompactScroll>
      )}
    </>
  );
}

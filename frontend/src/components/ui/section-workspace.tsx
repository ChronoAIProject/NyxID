import { useState, type ReactNode, type KeyboardEvent } from "react";
import { Search } from "lucide-react";
import { translateUi } from "@/i18n/use-ui-translation";
import { useSearchReveal } from "@/hooks/use-search-reveal";
import { normalizeDrawerSearch } from "@/lib/drawer-search";
import { GroupedDetailContext } from "@/lib/grouped-detail";
import { cn } from "@/lib/utils";

export interface SectionSearchItem<S extends string> {
  readonly id: string;
  readonly label: string;
  readonly setting?: string;
  readonly section: S;
  readonly hint: string;
  readonly target?: () => HTMLElement | null;
}

/** One retained content tree: wide sidebar navigation and narrow drill-ins share form owners. */
export function SectionWorkspace<S extends string>({
  wide,
  selected,
  groups,
  title,
  searchLabel,
  searchItems,
  navigate,
  children,
}: {
  readonly wide: boolean;
  readonly selected?: S;
  readonly groups: readonly (readonly {
    section: S;
    label: string;
    icon: ReactNode;
  }[])[];
  readonly title: string;
  readonly searchLabel: string;
  readonly searchItems: readonly SectionSearchItem<S>[];
  readonly navigate: (section: S, setting?: string) => void;
  readonly children: ReactNode;
}) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [query, setQuery] = useState("");
  const reveal = useSearchReveal(container);
  const words = normalizeDrawerSearch(query).trim().split(/\s+/);
  const matches = searchItems.filter((item) =>
    words.every((word) =>
      normalizeDrawerSearch(`${item.label} ${item.hint}`).includes(word),
    ),
  );
  const searching = Boolean(query.trim());
  function choose(section: S, item?: SectionSearchItem<S>) {
    navigate(section, item?.setting);
    reveal({
      id: item?.id ?? section,
      target:
        item?.target ??
        (() =>
          container?.querySelector<HTMLElement>(
            `[data-grouped-section="${section.split("/")[0]}"]:not([hidden]) h2`,
          ) ?? null),
    });
    setQuery("");
  }
  function move(event: KeyboardEvent<HTMLElement>) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = [
      ...event.currentTarget.querySelectorAll<HTMLButtonElement>("button"),
    ];
    if (!buttons.length) return;
    event.preventDefault();
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : (index + (event.key === "ArrowUp" ? -1 : 1) + buttons.length) %
            buttons.length;
    buttons[next]?.focus();
  }
  return (
    <div
      data-section-workspace=""
      data-wide={wide}
      className="flex min-h-0 flex-1"
    >
      {wide && (
        <nav
          aria-label={title}
          className="flex w-56 shrink-0 flex-col gap-4 border-r border-border/50 py-4 pr-4"
        >
          <div
            role="search"
            aria-label={searchLabel}
            className="flex min-h-9 items-center gap-2 rounded-lg bg-overlay-strong px-3"
          >
            <Search
              aria-hidden
              className="size-4 shrink-0 text-text-tertiary"
            />
            <input
              type="search"
              aria-label={searchLabel}
              placeholder={searchLabel}
              maxLength={200}
              value={query}
              className="min-w-0 flex-1 border-0 bg-transparent py-2 type-body outline-none placeholder:text-text-tertiary"
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape" && query) {
                  event.preventDefault();
                  event.stopPropagation();
                  setQuery("");
                }
                if (event.key === "Enter" && searching && matches[0]) {
                  event.preventDefault();
                  choose(matches[0].section, matches[0]);
                }
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  event.currentTarget
                    .closest("nav")
                    ?.querySelector<HTMLButtonElement>("button")
                    ?.focus();
                }
              }}
            />
          </div>
          <div
            className="assistant-scrollbar min-h-0 overflow-y-auto"
            onKeyDown={move}
          >
            {searching ? (
              matches.length ? (
                <ul className="space-y-1">
                  {matches.map((item) => (
                    <li key={item.id}>
                      <button
                        type="button"
                        className="min-h-11 w-full rounded-lg px-3 py-2 text-left type-nav-item text-muted-foreground hover:bg-overlay hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-focus"
                        onClick={() => choose(item.section, item)}
                      >
                        <span className="block break-words">{item.label}</span>
                        <span className="block type-secondary text-text-tertiary">
                          {item.hint}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="px-3 py-2 type-secondary text-text-tertiary">
                  {translateUi("No results found")}
                </p>
              )
            ) : (
              groups.map((group, index) => (
                <ul
                  key={index}
                  className={cn(
                    "space-y-1",
                    index > 0 && "mt-3 border-t border-border/50 pt-3",
                  )}
                >
                  {group.map((item) => (
                    <li key={item.section}>
                      <button
                        type="button"
                        aria-current={
                          selected?.split("/")[0] === item.section
                            ? "page"
                            : undefined
                        }
                        className={cn(
                          "flex min-h-11 w-full items-center gap-3 rounded-lg px-3 py-2 text-left",
                          selected?.split("/")[0] === item.section
                            ? "bg-overlay-strong type-active-nav-item text-foreground"
                            : "type-nav-item text-muted-foreground hover:bg-overlay hover:text-foreground",
                        )}
                        onClick={() => choose(item.section)}
                      >
                        {item.icon}
                        <span>{item.label}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              ))
            )}
          </div>
        </nav>
      )}
      <div
        ref={setContainer}
        data-section-pane=""
        className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      >
        <GroupedDetailContext.Provider value={{ container, visible: true }}>
          {children}
        </GroupedDetailContext.Provider>
      </div>
    </div>
  );
}

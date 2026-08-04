import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Search, X } from "lucide-react";
import { docTabForSlug } from "./manifest";

interface IndexEntry {
  readonly source: string;
  readonly title: string;
  readonly description: string;
  readonly headings: readonly string[];
}

const TAB_LABEL: Record<string, string> = {
  ai: "AI-assisted",
  web: "Web",
  cli: "CLI",
  shared: "Concepts",
};

export function DocsSearch({
  open,
  onClose,
}: {
  readonly open: boolean;
  readonly onClose: () => void;
}) {
  const [entries, setEntries] = useState<IndexEntry[]>([]);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const navigate = useNavigate();

  useEffect(() => {
    if (!open) return;
    if (entries.length === 0) {
      fetch("/docs/search-index.json")
        .then((r) => (r.ok ? r.json() : []))
        .then((d) => setEntries(Array.isArray(d) ? (d as IndexEntry[]) : []))
        .catch(() => {});
    }
    const id = window.setTimeout(() => {
      inputRef.current?.focus();
      setActiveIndex(0); // start each open at the top of the result list
    }, 20);
    return () => window.clearTimeout(id);
  }, [open, entries.length]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return entries
      .filter((e) => {
        const hay = `${e.title} ${e.description} ${e.headings.join(" ")} ${e.source}`.toLowerCase();
        return hay.includes(q);
      })
      .slice(0, 24);
  }, [query, entries]);

  // Keep the highlighted result scrolled into view as you arrow through.
  useEffect(() => {
    listRef.current?.children[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  if (!open) return null;

  const go = (slug: string) => {
    onClose();
    navigate({ to: "/docs/$", params: { _splat: slug } });
  };

  // ↑/↓ move the highlight (wrapping), Enter opens it. Handled on the input —
  // it always holds focus while the palette is open — so the keys never reach
  // the page behind. Escape is handled by the window listener above.
  const onInputKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (results.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => (i + 1) % results.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => (i - 1 + results.length) % results.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const r = results[Math.min(activeIndex, results.length - 1)];
      if (r) go(r.source.replace(/\.md$/, ""));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh] font-sans">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} aria-hidden />
      <div className="relative w-full max-w-xl overflow-hidden rounded-2xl border border-border bg-card shadow-2xl">
        <div className="flex items-center gap-3 border-b border-border px-4">
          <Search className="h-4 w-4 shrink-0 text-text-tertiary" aria-hidden />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActiveIndex(0); // a new query resets the highlight to the top
            }}
            onKeyDown={onInputKeyDown}
            placeholder="Search the docs…"
            role="combobox"
            aria-expanded={results.length > 0}
            aria-controls="docs-search-results"
            aria-activedescendant={
              results.length > 0 ? `docs-search-opt-${activeIndex}` : undefined
            }
            className="h-12 flex-1 bg-transparent text-sm text-foreground placeholder:text-text-tertiary focus:outline-none"
          />
          <button type="button" aria-label="Close search" onClick={onClose}>
            <X className="h-4 w-4 text-text-tertiary transition-colors hover:text-foreground" />
          </button>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-2">
          {query.trim() === "" ? (
            <p className="px-3 py-6 text-center text-sm text-text-tertiary">
              Type to search across CLI, Web, AI-assisted, and Concepts.
            </p>
          ) : results.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-text-tertiary">No results for “{query}”.</p>
          ) : (
            <ul className="space-y-0.5" id="docs-search-results" role="listbox" ref={listRef}>
              {results.map((e, i) => {
                const slug = e.source.replace(/\.md$/, "");
                const active = i === activeIndex;
                return (
                  <li key={slug} role="option" aria-selected={active} id={`docs-search-opt-${i}`}>
                    <Link
                      to="/docs/$"
                      params={{ _splat: slug }}
                      onClick={onClose}
                      onMouseEnter={() => setActiveIndex(i)}
                      className={`block rounded-lg px-3 py-2 transition-colors ${
                        active ? "bg-white/[0.06]" : "hover:bg-white/[0.04]"
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-[10px] tracking-wider text-nyx-secondary-400 uppercase">
                          {TAB_LABEL[docTabForSlug(slug)] ?? ""}
                        </span>
                        <span className="text-sm font-medium text-foreground">{e.title}</span>
                      </div>
                      {e.description && (
                        <p className="mt-0.5 line-clamp-1 text-xs text-text-tertiary">{e.description}</p>
                      )}
                    </Link>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        <div className="flex items-center gap-4 border-t border-border px-4 py-2 text-[11px] text-text-tertiary">
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border px-1 font-mono">↑</kbd>
            <kbd className="rounded border border-border px-1 font-mono">↓</kbd>
            navigate
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border px-1 font-mono">↵</kbd>
            open
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border px-1 font-mono">esc</kbd>
            close
          </span>
        </div>
      </div>
    </div>
  );
}

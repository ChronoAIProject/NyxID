import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
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
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    if (entries.length === 0) {
      fetch("/docs/search-index.json")
        .then((r) => (r.ok ? r.json() : []))
        .then((d) => setEntries(Array.isArray(d) ? (d as IndexEntry[]) : []))
        .catch(() => {});
    }
    const id = window.setTimeout(() => inputRef.current?.focus(), 20);
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

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh] font-sans">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} aria-hidden />
      <div className="relative w-full max-w-xl overflow-hidden rounded-2xl border border-border bg-card shadow-2xl">
        <div className="flex items-center gap-3 border-b border-border px-4">
          <Search className="h-4 w-4 shrink-0 text-text-tertiary" aria-hidden />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search the docs…"
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
            <ul className="space-y-0.5">
              {results.map((e) => {
                const slug = e.source.replace(/\.md$/, "");
                return (
                  <li key={slug}>
                    <Link
                      to="/docs/$"
                      params={{ _splat: slug }}
                      onClick={onClose}
                      className="block rounded-lg px-3 py-2 transition-colors hover:bg-white/[0.04]"
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
      </div>
    </div>
  );
}

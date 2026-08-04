import { useState, useCallback } from "react";
import { LayoutGrid, List } from "lucide-react";

export type ViewMode = "grid" | "table";

export function useViewMode(key: string): [ViewMode, (mode: ViewMode) => void] {
  const storageKey = `nyxid-view-mode:${key}`;

  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    try {
      const v = localStorage.getItem(storageKey);
      if (v === "table") return "table";
    } catch { /* SSR / private browsing */ }
    return "grid";
  });

  const setAndPersist = useCallback((mode: ViewMode) => {
    setViewMode(mode);
    try { localStorage.setItem(storageKey, mode); } catch { /* ignore */ }
  }, [storageKey]);

  return [viewMode, setAndPersist];
}

export function ViewToggle({
  viewMode,
  onViewModeChange,
}: {
  readonly viewMode: ViewMode;
  readonly onViewModeChange: (mode: ViewMode) => void;
}) {
  const toggle = () => onViewModeChange(viewMode === "grid" ? "table" : "grid");

  return (
    <div className="relative hidden items-center rounded-lg border border-border/50 p-0.5 md:flex">
      <div
        className="absolute top-0.5 h-7 w-7 rounded-md bg-white/[0.08] transition-transform duration-200 ease-out"
        style={{ transform: viewMode === "grid" ? "translateX(0)" : "translateX(100%)" }}
      />
      <button
        type="button"
        className="relative z-10 flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-colors"
        onClick={toggle}
        aria-label="Grid view"
        aria-pressed={viewMode === "grid"}
      >
        <LayoutGrid className={`h-3.5 w-3.5 transition-colors duration-200 ${viewMode === "grid" ? "text-foreground" : "text-text-tertiary hover:text-muted-foreground"}`} />
      </button>
      <button
        type="button"
        className="relative z-10 flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-colors"
        onClick={toggle}
        aria-label="Table view"
        aria-pressed={viewMode === "table"}
      >
        <List className={`h-3.5 w-3.5 transition-colors duration-200 ${viewMode === "table" ? "text-foreground" : "text-text-tertiary hover:text-muted-foreground"}`} />
      </button>
    </div>
  );
}

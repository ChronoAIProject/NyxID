import { AlertTriangle, LayoutDashboard } from "lucide-react";
import type { CrmSavedViewSummary } from "@/hooks/use-crm-views";
import { cn } from "@/lib/utils";

export function CrmViewList({
  views,
  selectedId,
  onSelect,
}: {
  readonly views: readonly CrmSavedViewSummary[];
  readonly selectedId?: string;
  readonly onSelect: (id: string) => void;
}) {
  if (views.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center">
        <LayoutDashboard className="mx-auto size-5 text-text-tertiary" />
        <p className="mt-2 text-12 font-medium text-muted-foreground">
          No saved views
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-1" role="list" aria-label="Saved CRM views">
      {views.map((view) => {
        const selected = view.id === selectedId;
        return (
          <div key={view.id} role="listitem">
            <button
              type="button"
              aria-current={selected ? "true" : undefined}
              onClick={() => onSelect(view.id)}
              className={cn(
                "flex w-full items-start gap-2 rounded-lg border px-3 py-2.5 text-left transition-colors",
                selected
                  ? "border-hairline-strong bg-overlay-strong"
                  : "border-transparent hover:border-hairline hover:bg-overlay",
              )}
            >
              <LayoutDashboard className="mt-0.5 size-3.5 shrink-0 text-text-tertiary" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-12 font-medium text-foreground">
                  {view.title}
                </span>
                <span className="mt-0.5 block text-10 text-text-tertiary">
                  Revision {view.revision.toLocaleString()}
                </span>
              </span>
              {!view.valid ? (
                <AlertTriangle
                  aria-label="View needs attention"
                  className="mt-0.5 size-3.5 shrink-0 text-warning"
                />
              ) : null}
            </button>
          </div>
        );
      })}
    </div>
  );
}

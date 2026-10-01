import { useState } from "react";
import { Bookmark, Check, ChevronDown, Save, Star } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { useServiceView } from "@/hooks/use-service-view";

export function ServiceSavedViews({
  view,
  onRestore,
}: {
  readonly view: ReturnType<typeof useServiceView>;
  readonly onRestore: () => void;
}) {
  const [open, setOpen] = useState(false);
  const source = {
    personal: "Personal",
    all: "All services",
    org: "Organization",
    platform: "NyxID platform",
  }[view.savedFilters.source];
  const savedDescription = [
    source,
    view.savedFilters.organization_ids.length
      ? `${view.savedFilters.organization_ids.length} organizations`
      : null,
    view.savedFilters.service_group_ids.length
      ? `${view.savedFilters.service_group_ids.length} services`
      : null,
    view.savedFilters.search ? `“${view.savedFilters.search}”` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            className="rounded-full"
            aria-label="Saved views"
          >
            <Bookmark className="size-3.5" aria-hidden="true" />
            Saved views
            <span className="rounded-full bg-muted px-1.5 text-[10px] tabular-nums">
              {view.hasDefault ? 1 : 0}
            </span>
            <ChevronDown className="size-3" aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-80 max-w-[calc(100vw-2rem)] space-y-3"
        >
          <div>
            <h3 className="text-[13px] font-semibold">Saved views</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              Your default opens when you return to AI Services.
            </p>
          </div>
          {view.hasDefault ? (
            <button
              type="button"
              className="flex w-full items-start gap-2 rounded-lg border border-border bg-muted/20 p-3 text-left hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
              aria-label="Restore default"
              onClick={() => {
                onRestore();
                setOpen(false);
              }}
            >
              <Star
                className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1">
                <span className="block text-xs font-medium">My default</span>
                <span className="mt-1 block break-words text-[11px] text-muted-foreground">
                  {savedDescription}
                </span>
              </span>
              {view.isDefault && (
                <Check
                  className="size-3.5 shrink-0 text-success"
                  aria-label="Active"
                />
              )}
            </button>
          ) : (
            <p className="rounded-lg border border-dashed border-border p-3 text-xs text-muted-foreground">
              No saved view yet. Set your filters, then save them as your
              default.
            </p>
          )}
          {!view.canSave && (
            <p className="text-[11px] text-muted-foreground">
              Saving account defaults requires the updated server.
            </p>
          )}
          <Button
            variant="outline"
            className="w-full"
            disabled={!view.canSave || view.isSaving || view.isDefault}
            isLoading={view.isSaving}
            onClick={view.saveDefault}
          >
            <Save className="size-3.5" aria-hidden="true" />
            {view.hasDefault
              ? "Update default with current filters"
              : "Save current filters as default"}
          </Button>
        </PopoverContent>
      </Popover>
      {view.isDefault ? (
        <span
          role="status"
          className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground"
        >
          <Check className="size-3 text-success" aria-hidden="true" />
          Default view
        </span>
      ) : (
        <Button
          variant="outline"
          size="sm"
          className="rounded-full"
          disabled={!view.canSave || view.isSaving}
          isLoading={view.isSaving}
          title={
            !view.canSave
              ? "Account defaults require the updated server."
              : "Save these filters to your account"
          }
          onClick={view.saveDefault}
        >
          <Save className="size-3" aria-hidden="true" />
          {view.hasDefault ? "Update default" : "Save as default"}
        </Button>
      )}
    </div>
  );
}

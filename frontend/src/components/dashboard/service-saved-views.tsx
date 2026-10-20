import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Bookmark, Check, ChevronDown, Save, Star, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  sameServiceFilters,
  serviceViewNameFormSchema,
  type ServiceViewFilters,
} from "@/schemas/service-view";
import type { useServiceView } from "@/hooks/use-service-view";

export function ServiceSavedViews({
  view,
  onRestore,
}: {
  readonly view: ReturnType<typeof useServiceView>;
  readonly onRestore: (filters?: ServiceViewFilters, viewId?: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const form = useAppForm<{ name: string }>({
    resolver: zodResolver(serviceViewNameFormSchema),
    defaultValues: { name: "" },
    mode: "onChange",
  });
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const chosen = view.workspace.views.find(
    (saved) => saved.id === view.selectedViewId,
  );
  const defaultView = view.workspace.views.find(
    (saved) => saved.id === view.workspace.default_id,
  );
  const active = [chosen, defaultView, ...view.workspace.views].find(
    (saved) => saved && sameServiceFilters(saved.filters, view.filters),
  );
  const selected = chosen ?? active;
  const disabled = !view.canSaveViews || view.isSaving;
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
            <span className="rounded-full bg-muted px-1.5 text-10 tabular-nums">
              {view.workspace.views.length}
            </span>
            <ChevronDown className="size-3" aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-96 max-w-[calc(100vw-2rem)] space-y-3"
        >
          <div>
            <h3 className="text-13 font-semibold">Saved views</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              Save named filter combinations. The default opens when you return
              to AI Services.
            </p>
          </div>
          <div className="max-h-64 space-y-2 overflow-y-auto">
            {view.workspace.views.map((saved) => (
              <div
                key={saved.id}
                className="rounded-lg border border-border p-2"
              >
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    className="min-w-0 flex-1 rounded p-1 text-left text-xs font-medium hover:bg-accent"
                    onClick={() => {
                      onRestore(saved.filters, saved.id);
                      setOpen(false);
                    }}
                  >
                    <span className="flex items-center gap-2">
                      <span className="truncate">{saved.name}</span>
                      {active?.id === saved.id && (
                        <Check
                          className="size-3 text-success"
                          aria-label="Active"
                        />
                      )}
                    </span>
                    <span className="mt-1 block text-11 font-normal text-muted-foreground">
                      {view.workspace.default_id === saved.id
                        ? "Default · "
                        : ""}
                      {
                        {
                          all: "All services",
                          personal: "Personal",
                          org: "Organization",
                          platform: "NyxID platform",
                        }[saved.filters.source]
                      }
                      {saved.filters.search
                        ? ` · “${saved.filters.search}”`
                        : ""}
                    </span>
                  </button>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={
                      view.workspace.default_id === saved.id
                        ? `Remove default: ${saved.name}`
                        : `Set as default: ${saved.name}`
                    }
                    title={
                      view.workspace.default_id === saved.id
                        ? "Remove default"
                        : "Set as default"
                    }
                    disabled={disabled}
                    onClick={() =>
                      view.setDefaultView(
                        view.workspace.default_id === saved.id
                          ? null
                          : saved.id,
                      )
                    }
                  >
                    <Star
                      className="size-3.5"
                      fill={
                        view.workspace.default_id === saved.id
                          ? "currentColor"
                          : "none"
                      }
                    />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`Delete view: ${saved.name}`}
                    disabled={disabled}
                    onClick={() => setPendingDelete(saved.id)}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
                {pendingDelete === saved.id && (
                  <div className="mt-2 flex items-center gap-2 text-xs">
                    <span>Delete this view?</span>
                    <Button
                      size="sm"
                      variant="destructive"
                      disabled={disabled}
                      onClick={() => {
                        view.deleteSavedView(saved.id);
                        setPendingDelete(null);
                      }}
                    >
                      Delete
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setPendingDelete(null)}
                    >
                      Cancel
                    </Button>
                  </div>
                )}
              </div>
            ))}
          </div>
          {view.workspace.views.length === 0 && (
            <p className="text-xs text-muted-foreground">
              No saved views yet. Set your filters, then save a view below.
            </p>
          )}
          {selected && (
            <Button
              variant="outline"
              className="w-full"
              disabled={
                disabled || sameServiceFilters(selected.filters, view.filters)
              }
              onClick={() => view.updateSavedView(selected.id)}
            >
              <Save className="size-3.5" />
              Update “{selected.name}” with current filters
            </Button>
          )}
          <form
            className="space-y-2 border-t border-border pt-3"
            onSubmit={form.handleSubmit(({ name }) => {
              if (!disabled && view.workspace.views.length < 20) {
                view.saveNewView(name, () => form.reset());
              }
            })}
          >
            <Label htmlFor="service-view-name">Save as a new view</Label>
            <Input
              id="service-view-name"
              {...form.register("name")}
              disabled={disabled}
              aria-invalid={!!form.formState.errors.name}
              aria-describedby={
                form.formState.errors.name
                  ? "service-view-name-error"
                  : undefined
              }
              placeholder="e.g. Team AI services"
            />
            {form.formState.errors.name && (
              <p
                id="service-view-name-error"
                className="text-xs text-destructive"
              >
                {form.formState.errors.name.message}
              </p>
            )}
            <Button
              type="submit"
              className="w-full"
              disabled={
                disabled ||
                !form.formState.isValid ||
                view.workspace.views.length >= 20
              }
              isLoading={view.isSaving}
            >
              Save new view
            </Button>
            {view.workspace.views.length >= 20 && (
              <p className="text-xs text-muted-foreground">
                You can save up to 20 views.
              </p>
            )}
          </form>
          {!view.canSaveViews && (
            <p className="text-11 text-muted-foreground">
              Saving named views requires the updated server.
            </p>
          )}
          {view.saveError && (
            <p role="alert" className="text-xs text-destructive">
              {view.saveError}
            </p>
          )}
        </PopoverContent>
      </Popover>
      {active && (
        <span
          role="status"
          className="inline-flex items-center gap-1.5 text-11 text-muted-foreground"
        >
          <Check className="size-3 text-success" />
          {active.name}
          {active.id === view.workspace.default_id ? " · Default" : ""}
        </span>
      )}
    </div>
  );
}

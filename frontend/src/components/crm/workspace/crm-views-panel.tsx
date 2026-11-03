import { useState } from "react";
import { Edit3, Plus, RefreshCw } from "lucide-react";
import { CrmViewEditor } from "@/components/crm/workspace/view-editor";
import { CrmViewList } from "@/components/crm/workspace/view-list";
import { CrmViewRenderer } from "@/components/crm/workspace/renderer";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useCrmView,
  useCrmViews,
  type CrmSavedView,
} from "@/hooks/use-crm-views";

export interface CrmViewsPanelProps {
  readonly ownerId?: string;
}

export function CrmViewsPanel({ ownerId }: CrmViewsPanelProps) {
  const [selectedId, setSelectedId] = useState<string>();
  const [editorMode, setEditorMode] = useState<"create" | "edit">();
  const views = useCrmViews(ownerId);
  const selected = useCrmView(selectedId, ownerId);

  async function reloadSelected() {
    await selected.refetch();
    setEditorMode(undefined);
  }

  function handleSaved(view: CrmSavedView) {
    setSelectedId(view.id);
    setEditorMode(undefined);
  }

  return (
    <section aria-labelledby="crm-views-heading" className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 id="crm-views-heading" className="text-15 font-semibold">
            Views
          </h2>
          <p className="mt-1 text-12 text-muted-foreground">
            Saved CRM workspaces
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            size="icon"
            variant="outline"
            onClick={() => void views.refetch()}
            aria-label="Refresh saved views"
            title="Refresh saved views"
          >
            <RefreshCw className={views.isFetching ? "animate-spin" : undefined} />
          </Button>
          <Button
            type="button"
            onClick={() => setEditorMode("create")}
          >
            <Plus />
            New view
          </Button>
        </div>
      </div>

      {views.isError ? (
        <ErrorBanner
          message="Could not load saved views."
          onRetry={() => void views.refetch()}
        />
      ) : (
        <div className="grid min-w-0 gap-4 lg:grid-cols-[14rem_minmax(0,1fr)]">
          <aside className="rounded-xl border border-border/50 bg-card p-2 shadow-sm">
            {views.isPending ? (
              <p role="status" className="px-3 py-8 text-center text-12 text-text-tertiary">
                Loading views...
              </p>
            ) : (
              <CrmViewList
                views={views.data ?? []}
                selectedId={selectedId}
                onSelect={(id) => {
                  setSelectedId(id);
                  setEditorMode(undefined);
                }}
              />
            )}
          </aside>

          <div className="min-w-0">
            {editorMode === "create" ? (
              <CrmViewEditor
                ownerId={ownerId}
                onSaved={handleSaved}
                onCancel={() => setEditorMode(undefined)}
              />
            ) : null}

            {editorMode === "edit" && selected.data ? (
              <CrmViewEditor
                key={`${selected.data.id}:${String(selected.data.revision)}`}
                view={selected.data}
                ownerId={ownerId}
                onSaved={handleSaved}
                onCancel={() => setEditorMode(undefined)}
                onReload={() => void reloadSelected()}
              />
            ) : null}

            {!editorMode && !selectedId ? (
              <div className="flex min-h-48 items-center justify-center rounded-xl border border-dashed border-border text-12 text-text-tertiary">
                Select a saved view
              </div>
            ) : null}

            {!editorMode && selectedId && selected.isPending ? (
              <div
                role="status"
                className="flex min-h-48 items-center justify-center rounded-xl border border-border/50 text-12 text-text-tertiary"
              >
                Loading view...
              </div>
            ) : null}

            {!editorMode && selected.isError ? (
              <ErrorBanner
                message="Could not open this view."
                onRetry={() => void selected.refetch()}
              />
            ) : null}

            {!editorMode && selected.data ? (
              <div className="space-y-4">
                <div className="flex items-start justify-between gap-3 border-b border-border/50 pb-3">
                  <div>
                    <h3 className="text-15 font-semibold">
                      {selected.data.title}
                    </h3>
                    <p className="mt-1 text-11 text-text-tertiary">
                      Revision {selected.data.revision.toLocaleString()}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setEditorMode("edit")}
                  >
                    <Edit3 />
                    Edit
                  </Button>
                </div>
                <CrmViewRenderer
                  envelope={selected.data.envelope}
                  ownerId={ownerId}
                  valid={selected.data.valid}
                  errors={selected.data.errors}
                />
              </div>
            ) : null}
          </div>
        </div>
      )}
    </section>
  );
}

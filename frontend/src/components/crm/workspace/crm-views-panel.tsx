import { useAuthStore } from "@/stores/auth-store";
import { useCrmLocation } from "@/lib/crm/location";
import { crmLink } from "@/lib/crm/record-navigation";
import { crmAuthorityRefused } from "@/lib/crm/query-visibility";
import { useRef, useState } from "react";
import { Edit3, Plus, RefreshCw, Trash2 } from "lucide-react";
import { CrmViewEditor } from "@/components/crm/workspace/view-editor";
import { CrmViewList } from "@/components/crm/workspace/view-list";
import { CrmViewRenderer } from "@/components/crm/workspace/renderer";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useDeleteCrmView,
  useCrmView,
  useCrmViews,
  type CrmSavedView,
} from "@/hooks/use-crm-views";
import { useCrmObjects } from "@/hooks/use-crm";

export interface CrmViewsPanelProps {
  readonly ownerId?: string;
}

export function CrmViewsPanel({ ownerId }: CrmViewsPanelProps) {
  const location = useCrmLocation();
  const setCrmLocation = location.set;
  const selectedId = location.get("view") ?? undefined;
  const setSelectedId = (view: string | undefined) =>
    setCrmLocation({ view, record: undefined });
  const [editorMode, setEditorMode] = useState<"create" | "edit">();
  const objects = useCrmObjects(ownerId);
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const views = useCrmViews(ownerId, cursors.at(-1));
  const selected = useCrmView(selectedId, ownerId);
  const access = objects.data?.[0]?.access;
  const canWork = access === "work" || access === "schema";
  const actor = useAuthStore((state) => state.user?.id);
  const deletion = useDeleteCrmView(ownerId);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteError, setDeleteError] = useState<string>();
  const deleteKeys = useRef(new Map<string, string>());
  const canDelete =
    canWork &&
    (access === "schema" || selected.data?.created_by?.user_id === actor);
  async function deleteSelected() {
    if (!selected.data || !canDelete) return;
    const id = selected.data.id;
    const key = deleteKeys.current.get(id) ?? crypto.randomUUID();
    deleteKeys.current.set(id, key);
    setDeleteError(undefined);
    try {
      await deletion.mutateAsync({ id, idempotencyKey: key });
      deleteKeys.current.delete(id);
      setSelectedId(undefined);
      setConfirmDelete(false);
    } catch (cause) {
      setDeleteError(
        cause instanceof Error ? cause.message : "Could not delete view.",
      );
    }
  }

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
          <h2 id="crm-views-heading" className="type-card-title">
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
            <RefreshCw
              className={views.isFetching ? "animate-spin" : undefined}
            />
          </Button>
          {canWork ? (
            <Button type="button" onClick={() => setEditorMode("create")}>
              <Plus />
              New view
            </Button>
          ) : null}
        </div>
      </div>

      {views.isError && (!views.data || crmAuthorityRefused(views.error)) ? (
        <ErrorBanner
          message="Could not load saved views."
          onRetry={() => void views.refetch()}
        />
      ) : (
        <div className="grid min-w-0 gap-4 lg:grid-cols-[14rem_minmax(0,1fr)]">
          <aside className="rounded-xl border border-border/50 bg-card p-2 shadow-sm">
            {views.isPending ? (
              <p
                role="status"
                className="px-3 py-8 text-center text-12 text-text-tertiary"
              >
                Loading views...
              </p>
            ) : (
              <CrmViewList
                views={views.data?.views ?? []}
                selectedId={selectedId}
                onSelect={(id) => {
                  setSelectedId(id);
                  setEditorMode(undefined);
                  setConfirmDelete(false);
                }}
              />
            )}
            <div className="flex gap-2 p-2">
              <Button
                size="sm"
                variant="outline"
                disabled={cursors.length < 2}
                onClick={() => setCursors((items) => items.slice(0, -1))}
              >
                Previous views
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!views.data?.next_cursor}
                onClick={() =>
                  setCursors((items) => [
                    ...items,
                    views.data?.next_cursor ?? undefined,
                  ])
                }
              >
                Next views
              </Button>
            </div>
          </aside>

          <div className="min-w-0">
            {editorMode === "create" && canWork ? (
              <CrmViewEditor
                ownerId={ownerId}
                objects={objects.data}
                onSaved={handleSaved}
                onCancel={() => setEditorMode(undefined)}
              />
            ) : null}

            {editorMode === "edit" &&
            canWork &&
            !crmAuthorityRefused(selected.error) &&
            selected.data ? (
              <CrmViewEditor
                key={`${ownerId ?? "personal"}:${selected.data.id}`}
                view={selected.data}
                ownerId={ownerId}
                objects={objects.data}
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

            {!editorMode &&
            !crmAuthorityRefused(selected.error) &&
            selected.data ? (
              <div className="space-y-4">
                <div className="flex items-start justify-between gap-3 border-b border-border/50 pb-3">
                  <div>
                    <h3 className="type-card-title">
                      {selected.data.title}
                    </h3>
                    <a
                      className="type-meta underline"
                      href={crmLink({ ownerId, viewId: selected.data.id })}
                    >
                      Link to this view
                    </a>
                    <p className="mt-1 type-meta text-text-tertiary">
                      Revision {selected.data.revision.toLocaleString()}
                    </p>
                  </div>
                  {canWork ? (
                    <div className="flex gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => setEditorMode("edit")}
                      >
                        <Edit3 />
                        Edit
                      </Button>
                      {canDelete ? (
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => setConfirmDelete(true)}
                        >
                          <Trash2 />
                          Delete view
                        </Button>
                      ) : null}
                    </div>
                  ) : null}
                </div>
                {confirmDelete && canDelete ? (
                  <div
                    role="alert"
                    className="space-y-2 rounded-lg border border-border p-3"
                  >
                    <p>
                      Delete “{selected.data.title}”? Its records remain
                      available.
                    </p>
                    <Button
                      type="button"
                      variant="destructive"
                      disabled={deletion.isPending}
                      onClick={() => void deleteSelected()}
                    >
                      Confirm delete view
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      disabled={deletion.isPending}
                      onClick={() => {
                        setConfirmDelete(false);
                        setDeleteError(undefined);
                      }}
                    >
                      Cancel deletion
                    </Button>
                    {deleteError ? <p>{deleteError}</p> : null}
                  </div>
                ) : null}
                <CrmViewRenderer
                  recordId={location.get("record") ?? undefined}
                  onRecordChange={(record) => setCrmLocation({ record })}
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

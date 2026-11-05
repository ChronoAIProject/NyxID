import { useNyxAgentLive } from "@/hooks/use-nyxagent-live";
import { CrmOwnerContext } from "@/lib/crm/owner-context";
import { CrmSchemaEditor } from "@/components/crm/schema-editor";
import { CrmImportExport } from "@/components/crm/import-export";
import { CrmDateReport } from "@/components/crm/date-report";
import { useCrmLocation } from "@/lib/crm/location";
import { useOrgs } from "@/hooks/use-orgs";
import { useState } from "react";
import { Columns3, List, Plus } from "lucide-react";
import { CrmObjectTable } from "@/components/crm/object-table";
import { CrmPipelineBoard } from "@/components/crm/pipeline-board";
import {
  CrmCreateRecordSheet,
  CrmRecordSheet,
} from "@/components/crm/record-sheet";
import { CrmViewsPanel } from "@/components/crm/workspace/crm-views-panel";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useCrmObjects } from "@/hooks/use-crm";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { CrmTableQuery } from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";

function CrmWorkspace({ ownerId }: { readonly ownerId?: string }) {
  const location = useCrmLocation();
  const setCrmLocation = location.set;
  useNyxAgentLive(true);
  const objects = useCrmObjects(ownerId);
  const selectedKey = location.get("object") ?? undefined;
  const setSelectedKey = (object: string) =>
    setCrmLocation({ object, record: undefined });
  const [mode, setMode] = useState<"table" | "pipeline">("table");
  const surface =
    location.has("view") || location.get("surface") === "views"
      ? "views"
      : "records";
  const setSurface = (next: "records" | "views") =>
    setCrmLocation({
      surface: next === "views" ? "views" : undefined,
      view: undefined,
      record: undefined,
    });
  const [panel, setPanel] = useState<"schema" | "transfer" | "report">();
  const [includeArchived, setIncludeArchived] = useState(false);
  const [exportQuery, setExportQuery] = useState<CrmTableQuery>();
  const recordId = location.get("record") ?? undefined;
  const setRecordId = (record: string | undefined) =>
    setCrmLocation({ record });
  const [createOpen, setCreateOpen] = useState(false);
  const object =
    objects.data?.find((candidate) => candidate.key === selectedKey) ??
    objects.data?.[0];
  const effectiveMode =
    mode === "pipeline" && !object?.pipeline ? "table" : mode;
  const query: CrmTableQuery | undefined = object
    ? {
        object: object.key,
        select: object.default_columns.filter((key) =>
          object.fields.some((field) => field.key === key && !field.archived),
        ),
        filters: [],
        page_size: 50,
        include_archived: includeArchived && effectiveMode === "table",
      }
    : undefined;

  return (
    <div className="space-y-6">
      <PageHeader
        title="CRM"
        description="Companies, contacts, opportunities, and follow-up work."
        actions={
          surface === "records" && object && object.access !== "read" ? (
            <AddCtaButton
              label={`New ${object.label.toLocaleLowerCase()}`}
              icon={Plus}
              onClick={() => setCreateOpen(true)}
            />
          ) : undefined
        }
      />

      <div
        className="inline-flex w-fit rounded-lg border border-hairline bg-overlay p-0.5"
        role="group"
        aria-label="CRM workspace"
      >
        <Button
          type="button"
          size="sm"
          variant={surface === "records" ? "secondary" : "ghost"}
          aria-pressed={surface === "records"}
          onClick={() => setSurface("records")}
        >
          Records
        </Button>
        <Button
          type="button"
          size="sm"
          variant={surface === "views" ? "secondary" : "ghost"}
          aria-pressed={surface === "views"}
          onClick={() => setSurface("views")}
        >
          Saved views
        </Button>
      </div>

      {surface === "views" ? <CrmViewsPanel ownerId={ownerId} /> : null}

      {surface === "records" && objects.isPending ? (
        <p role="status" className="text-12 text-text-tertiary">
          Loading CRM objects...
        </p>
      ) : null}
      {surface === "records" && objects.isError ? (
        <div className="space-y-2" role="alert">
          <p className="text-12 text-destructive">
            Could not load CRM objects.
          </p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void objects.refetch()}
          >
            Retry
          </Button>
        </div>
      ) : null}

      {surface === "records" && object && query ? (
        <>
          <div className="flex flex-col gap-3 border-b border-border/50 pb-4 sm:flex-row sm:items-center sm:justify-between">
            <Select
              value={object.key}
              onValueChange={(key) => {
                setSelectedKey(key);
                setMode("table");
                setRecordId(undefined);
              }}
            >
              <SelectTrigger className="w-full sm:w-56" aria-label="CRM object">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {objects.data?.map((candidate) => (
                  <SelectItem key={candidate.key} value={candidate.key}>
                    {candidate.plural_label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div
              className="inline-flex w-fit rounded-lg border border-hairline bg-overlay p-0.5"
              role="group"
              aria-label="CRM view"
            >
              <Button
                type="button"
                size="sm"
                variant={effectiveMode === "table" ? "secondary" : "ghost"}
                aria-pressed={effectiveMode === "table"}
                onClick={() => setMode("table")}
              >
                <List />
                Table
              </Button>
              {object.pipeline ? (
                <Button
                  type="button"
                  size="sm"
                  variant={effectiveMode === "pipeline" ? "secondary" : "ghost"}
                  aria-pressed={effectiveMode === "pipeline"}
                  onClick={() => setMode("pipeline")}
                >
                  <Columns3 />
                  Pipeline
                </Button>
              ) : null}
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {effectiveMode === "table" ? (
              <label className="flex gap-2 text-12">
                <input
                  type="checkbox"
                  checked={includeArchived}
                  onChange={(event) => setIncludeArchived(event.target.checked)}
                />
                Include archived records
              </label>
            ) : null}
            {object.access === "schema" ? (
              <Button
                variant="outline"
                size="sm"
                aria-pressed={panel === "schema"}
                onClick={() =>
                  setPanel(panel === "schema" ? undefined : "schema")
                }
              >
                Schema settings
              </Button>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              aria-pressed={panel === "transfer"}
              onClick={() =>
                setPanel(panel === "transfer" ? undefined : "transfer")
              }
            >
              Import / export
            </Button>
            <Button
              variant="outline"
              size="sm"
              aria-pressed={panel === "report"}
              onClick={() =>
                setPanel(panel === "report" ? undefined : "report")
              }
            >
              Dated report
            </Button>
          </div>
          {panel === "schema" && object.access === "schema" ? (
            <CrmSchemaEditor
              key={object.key}
              object={object}
              objects={objects.data}
              ownerId={ownerId}
            />
          ) : null}
          {panel === "transfer" ? (
            <CrmImportExport
              key={JSON.stringify(query)}
              object={object}
              query={
                effectiveMode === "table" &&
                exportQuery?.object === query.object
                  ? exportQuery
                  : query
              }
              ownerId={ownerId}
            />
          ) : null}
          {panel === "report" ? (
            <CrmDateReport
              key={object.key}
              object={object}
              query={
                effectiveMode === "table" &&
                exportQuery?.object === query.object
                  ? exportQuery
                  : query
              }
              ownerId={ownerId}
            />
          ) : null}
          {effectiveMode === "pipeline" ? (
            <CrmPipelineBoard
              ownerId={ownerId}
              query={query}
              onSelectRecord={setRecordId}
            />
          ) : (
            <CrmObjectTable
              ownerId={ownerId}
              onQueryChange={setExportQuery}
              query={query}
              onSelectRecord={setRecordId}
            />
          )}

          <CrmCreateRecordSheet
            ownerId={ownerId}
            object={object}
            open={createOpen}
            onOpenChange={setCreateOpen}
            onCreated={setRecordId}
          />
          <CrmRecordSheet
            ownerId={ownerId}
            recordId={recordId}
            open={Boolean(recordId)}
            onOpenChange={(open) => {
              if (!open) setRecordId(undefined);
            }}
            onSelectRecord={setRecordId}
          />
        </>
      ) : null}
    </div>
  );
}

function EnabledCrmPage() {
  const actor = useAuthStore((state) => state.user?.id);
  const location = useCrmLocation();
  const orgs = useOrgs();
  const owner = location.get("owner") ?? actor;
  return (
    <CrmOwnerContext value={owner}>
      <div className="space-y-4">
        <label className="flex items-center gap-3 text-12">
          CRM owner
          <select
            aria-label="CRM owner"
            className="rounded-lg border border-border bg-background p-2"
            value={owner ?? ""}
            onChange={(event) =>
              location.set({
                owner: event.target.value,
                record: undefined,
                view: undefined,
                object: undefined,
                surface: undefined,
              })
            }
          >
            <option value={actor}>Personal workspace</option>
            {orgs.data?.map((org) => (
              <option key={org.id} value={org.id}>
                {org.display_name ?? org.slug}
              </option>
            ))}
          </select>
        </label>
        <CrmWorkspace key={`${actor}:${owner}`} ownerId={owner} />
      </div>
    </CrmOwnerContext>
  );
}
export function CrmPage() {
  const loading = useAuthStore((state) => state.isLoading);
  const enabled = useFeature(FEATURE_FLAG.CMS_POC_V1);
  if (loading) return null;
  if (!enabled)
    return (
      <p className="text-13 text-text-secondary">
        CRM isn't available for your account.
      </p>
    );
  return <EnabledCrmPage />;
}

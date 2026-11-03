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

function CrmWorkspace() {
  const objects = useCrmObjects();
  const [selectedKey, setSelectedKey] = useState<string>();
  const [mode, setMode] = useState<"table" | "pipeline">("table");
  const [surface, setSurface] = useState<"records" | "views">("records");
  const [recordId, setRecordId] = useState<string>();
  const [createOpen, setCreateOpen] = useState(false);
  const object =
    objects.data?.find((candidate) => candidate.key === selectedKey) ??
    objects.data?.[0];
  const effectiveMode =
    mode === "pipeline" && !object?.pipeline ? "table" : mode;
  const query: CrmTableQuery | undefined = object
    ? {
        object: object.key,
        select: object.default_columns,
        filters: [],
        page_size: 50,
        include_archived: false,
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

      {surface === "views" ? <CrmViewsPanel /> : null}

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

          {effectiveMode === "pipeline" ? (
            <CrmPipelineBoard query={query} onSelectRecord={setRecordId} />
          ) : (
            <CrmObjectTable query={query} onSelectRecord={setRecordId} />
          )}

          <CrmCreateRecordSheet
            object={object}
            open={createOpen}
            onOpenChange={setCreateOpen}
            onCreated={setRecordId}
          />
          <CrmRecordSheet
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

export function CrmPage() {
  const loading = useAuthStore((state) => state.isLoading);
  const enabled = useFeature(FEATURE_FLAG.CRM);

  if (loading) return null;
  if (!enabled) {
    return (
      <p className="text-13 text-text-secondary">
        CRM isn't available for your account.
      </p>
    );
  }
  return <CrmWorkspace />;
}

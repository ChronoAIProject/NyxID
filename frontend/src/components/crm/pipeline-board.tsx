import {
  useCrmAggregate,
  useCrmObject,
  useCrmTableQuery,
} from "@/hooks/use-crm";
import type { CrmField, CrmTableQuery } from "@/schemas/crm";

export interface CrmPipelineBoardProps {
  readonly query: CrmTableQuery;
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}

function PipelineLane({
  query,
  stageField,
  stage,
  count,
  ownerId,
  onSelectRecord,
}: {
  readonly query: CrmTableQuery;
  readonly stageField: CrmField & { type: "select" };
  readonly stage: { readonly value: string; readonly label: string };
  readonly count: number;
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}) {
  const lane = useCrmTableQuery(
    {
      ...query,
      cursor: undefined,
      page_size: Math.min(query.page_size ?? 50, 50),
      filters: [
        ...query.filters.filter((filter) => filter.field !== stageField.key),
        { field: stageField.key, op: "eq", value: stage.value },
      ],
    },
    ownerId,
  );
  const titleField = lane.data?.object.fields.find(
    (field) => field.type === "text" && field.required,
  );

  return (
    <section
      className="min-w-64 flex-1 rounded-lg border border-border/50 bg-card"
      aria-label={stage.label}
    >
      <header className="flex items-center justify-between border-b border-border/50 px-3 py-2.5">
        <h3 className="text-12 font-medium">{stage.label}</h3>
        <span className="text-11 tabular-nums text-text-tertiary">
          {count.toLocaleString()}
        </span>
      </header>
      <div className="min-h-24 space-y-2 p-2">
        {lane.isPending ? (
          <p role="status" className="p-2 text-11 text-text-tertiary">
            Loading...
          </p>
        ) : null}
        {lane.isError ? (
          <p role="alert" className="p-2 text-11 text-destructive">
            Could not load this lane.
          </p>
        ) : null}
        {lane.data?.rows.map((record) => (
          <button
            key={record.id}
            type="button"
            onClick={
              onSelectRecord ? () => onSelectRecord(record.id) : undefined
            }
            className="block w-full rounded-lg border border-hairline bg-overlay p-3 text-left text-12 transition-colors hover:border-hairline-strong hover:bg-overlay-strong"
          >
            {titleField
              ? String(record.values[titleField.key] ?? "Untitled")
              : "Untitled"}
          </button>
        ))}
        {lane.data?.rows.length === 0 ? (
          <p className="p-2 text-center text-11 text-text-tertiary">
            No records
          </p>
        ) : null}
      </div>
    </section>
  );
}

export function CrmPipelineBoard({
  query,
  ownerId,
  onSelectRecord,
}: CrmPipelineBoardProps) {
  const object = useCrmObject(query.object, ownerId);
  const pipeline = object.data?.pipeline;
  const field = pipeline
    ? object.data?.fields.find(
        (candidate) => candidate.key === pipeline.stage_field,
      )
    : undefined;
  const stageField = field?.type === "select" ? field : undefined;
  const counts = useCrmAggregate(
    stageField
      ? {
          object: query.object,
          filters: query.filters.filter(
            (filter) => filter.field !== stageField.key,
          ),
          group_by: stageField.key,
          measure: { op: "count" },
          order: "option_order",
        }
      : undefined,
    ownerId,
    Boolean(stageField),
  );

  if (object.isPending) {
    return (
      <p role="status" className="text-12 text-text-tertiary">
        Loading pipeline...
      </p>
    );
  }
  if (object.isError) {
    return (
      <p role="alert" className="text-12 text-destructive">
        Could not load the pipeline.
      </p>
    );
  }
  if (!stageField) {
    return (
      <p className="text-12 text-text-tertiary">
        This object does not have a pipeline.
      </p>
    );
  }

  const countByStage = new Map(
    counts.data?.points.map((point) => [point.key, point.value]) ?? [],
  );
  return (
    <div className="flex gap-3 overflow-x-auto pb-2">
      {stageField.options
        .filter((option) => !option.archived)
        .map((stage) => (
          <PipelineLane
            key={stage.value}
            query={query}
            stageField={stageField}
            stage={stage}
            count={countByStage.get(stage.value) ?? 0}
            ownerId={ownerId}
            onSelectRecord={onSelectRecord}
          />
        ))}
    </div>
  );
}

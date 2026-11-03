import { useState } from "react";
import {
  DndContext,
  PointerSensor,
  KeyboardSensor,
  closestCorners,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, MoreHorizontal } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  useCrmAggregate,
  useCrmObject,
  useCrmTableQuery,
  useMoveCrmRecord,
} from "@/hooks/use-crm";
import {
  applyOptimisticPipelineRows,
  type CrmOptimisticPipelineMove,
} from "@/lib/crm/pipeline-optimistic";
import type { CrmField, CrmRecordRow, CrmTableQuery } from "@/schemas/crm";

export interface CrmPipelineBoardProps {
  readonly query: CrmTableQuery;
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}

function PipelineCard({
  record,
  stage,
  title,
  stages,
  pending,
  readOnly,
  onMove,
  onSelect,
}: {
  readonly record: CrmRecordRow;
  readonly stage: string;
  readonly title: string;
  readonly stages: readonly {
    readonly value: string;
    readonly label: string;
  }[];
  readonly pending: boolean;
  readonly readOnly: boolean;
  readonly onMove: (to: string) => void;
  readonly onSelect?: () => void;
}) {
  const { setNodeRef, transform, attributes, listeners } = useDraggable({
    id: record.id,
    disabled: pending || readOnly,
    data: { record, from: stage },
  });
  return (
    <article
      ref={setNodeRef}
      style={{ transform: CSS.Translate.toString(transform) }}
      className="flex items-start gap-1 rounded-lg border border-hairline bg-overlay p-2 text-12 shadow-sm transition-colors hover:border-hairline-strong hover:bg-overlay-strong"
      data-pending={pending || undefined}
    >
      <button
        type="button"
        aria-label={`Move ${title}`}
        className="mt-0.5 flex min-h-control cursor-grab items-center rounded-md px-1 text-text-tertiary outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed"
        disabled={pending || readOnly}
        {...attributes}
        {...listeners}
      >
        <GripVertical />
      </button>
      <button
        type="button"
        className="min-h-control min-w-0 flex-1 px-1 text-left font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onSelect}
      >
        <span className="block break-words">{title}</span>
        {pending ? (
          <span className="mt-1 block text-10 font-normal text-text-tertiary">
            Moving...
          </span>
        ) : null}
      </button>
      {!readOnly ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={`Move ${title} to another stage`}
              disabled={pending}
            >
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {stages
              .filter((candidate) => candidate.value !== stage)
              .map((candidate) => (
                <DropdownMenuItem
                  key={candidate.value}
                  onSelect={() => onMove(candidate.value)}
                >
                  Move to {candidate.label}
                </DropdownMenuItem>
              ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </article>
  );
}

function PipelineLane({
  query,
  stageField,
  stage,
  stageValues,
  count,
  ownerId,
  moves,
  onMove,
  onSelectRecord,
  readOnly,
}: {
  readonly query: CrmTableQuery;
  readonly stageField: CrmField & { type: "select" };
  readonly stage: { readonly value: string; readonly label: string };
  readonly stageValues?: readonly string[];
  readonly count: number;
  readonly ownerId?: string;
  readonly moves: Readonly<Record<string, CrmOptimisticPipelineMove>>;
  readonly onMove: (record: CrmRecordRow, from: string, to: string) => void;
  readonly onSelectRecord?: (recordId: string) => void;
  readonly readOnly: boolean;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: stage.value });
  const lane = useCrmTableQuery(
    {
      ...query,
      cursor: undefined,
      page_size: Math.min(query.page_size ?? 50, 50),
      filters: [
        ...query.filters.filter((filter) => filter.field !== stageField.key),
        stageValues && stageValues.length > 1
          ? { field: stageField.key, op: "in", value: stageValues }
          : {
              field: stageField.key,
              op: "eq",
              value: stageValues?.[0] ?? stage.value,
            },
      ],
    },
    ownerId,
  );
  const titleField = lane.data?.object.fields.find(
    (field) => field.type === "text" && field.required,
  );
  const rows = applyOptimisticPipelineRows(
    lane.data?.rows ?? [],
    stage.value,
    moves,
  );
  return (
    <section
      ref={setNodeRef}
      className="min-w-64 flex-1 rounded-lg border border-border/50 bg-card data-[over=true]:border-input-focus"
      data-over={isOver || undefined}
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
        {rows.map((record) => {
          const title = titleField
            ? String(record.values[titleField.key] ?? "Untitled")
            : "Untitled";
          const storedStage = record.values[stageField.key];
          const currentStage =
            moves[record.id]?.to ??
            (typeof storedStage === "string" ? storedStage : stage.value);
          return (
            <PipelineCard
              key={record.id}
              record={record}
              stage={currentStage}
              title={title}
              stages={stageField.options.filter((option) => !option.archived)}
              pending={Boolean(moves[record.id])}
              readOnly={readOnly}
              onMove={(to) => onMove(record, currentStage, to)}
              onSelect={
                onSelectRecord ? () => onSelectRecord(record.id) : undefined
              }
            />
          );
        })}
        {!lane.isPending && rows.length === 0 ? (
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
  const move = useMoveCrmRecord(ownerId);
  const [moves, setMoves] = useState<Record<string, CrmOptimisticPipelineMove>>(
    {},
  );
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor),
  );

  async function moveRecord(record: CrmRecordRow, from: string, to: string) {
    if (from === to || moves[record.id]) return;
    setMoves((current) => ({ ...current, [record.id]: { record, from, to } }));
    try {
      await move.mutateAsync({
        id: record.id,
        expectedRevision: record.revision,
        to,
        idempotencyKey: crypto.randomUUID(),
      });
    } catch (error) {
      toast.error("Could not move the record", {
        description:
          error instanceof Error
            ? error.message
            : "The pipeline was restored. Try again.",
      });
    } finally {
      setMoves((current) => {
        const next = { ...current };
        delete next[record.id];
        return next;
      });
    }
  }

  function dragEnd(event: DragEndEvent) {
    const to = typeof event.over?.id === "string" ? event.over.id : null;
    const data = event.active.data.current as
      | { record?: CrmRecordRow; from?: string }
      | undefined;
    if (to && data?.record && data.from)
      void moveRecord(data.record, data.from, to);
  }

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
  const countByStage = new Map<string, number>(
    counts.data?.points.flatMap((point) =>
      typeof point.key === "string" && typeof point.value === "number"
        ? [[point.key, point.value] as const]
        : [],
    ) ?? [],
  );
  for (const pending of Object.values(moves)) {
    countByStage.set(
      pending.from,
      Math.max(0, (countByStage.get(pending.from) ?? 0) - 1),
    );
    countByStage.set(pending.to, (countByStage.get(pending.to) ?? 0) + 1);
  }
  const liveStages = stageField.options.filter((option) => !option.archived);
  const archivedStages = stageField.options.filter((option) => option.archived);
  const liveCount = liveStages.reduce(
    (total, stage) => total + (countByStage.get(stage.value) ?? 0),
    0,
  );
  const archivedCount = Math.max(
    0,
    (typeof counts.data?.total === "number" ? counts.data.total : liveCount) -
      liveCount,
  );
  const readOnly = object.data?.access === "read";
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragEnd={dragEnd}
    >
      <div className="flex gap-3 overflow-x-auto pb-2">
        {liveStages.map((stage) => (
          <PipelineLane
            key={stage.value}
            query={query}
            stageField={stageField}
            stage={stage}
            count={countByStage.get(stage.value) ?? 0}
            ownerId={ownerId}
            moves={moves}
            onMove={(record, from, to) => void moveRecord(record, from, to)}
            onSelectRecord={onSelectRecord}
            readOnly={readOnly}
          />
        ))}
        {archivedStages.length > 0 || archivedCount > 0 ? (
          <PipelineLane
            query={query}
            stageField={stageField}
            stage={{ value: "__archived_stages__", label: "Archived stages" }}
            stageValues={archivedStages.map((stage) => stage.value)}
            count={archivedCount}
            ownerId={ownerId}
            moves={moves}
            onMove={(record, from, to) => void moveRecord(record, from, to)}
            onSelectRecord={onSelectRecord}
            readOnly
          />
        ) : null}
      </div>
    </DndContext>
  );
}

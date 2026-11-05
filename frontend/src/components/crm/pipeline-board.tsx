import { useQueryClient } from "@tanstack/react-query";
import { ApiError } from "@/lib/api-client";
import { useRef, useState } from "react";
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
import {
  crmPipelineCountQuery,
  crmPipelineLaneQuery,
  crmPipelineStageMatches,
} from "@/lib/crm/pipeline-query";
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
  readonly count?: number;
  readonly ownerId?: string;
  readonly moves: Readonly<Record<string, CrmOptimisticPipelineMove>>;
  readonly onMove: (record: CrmRecordRow, from: string, to: string) => void;
  readonly onSelectRecord?: (recordId: string) => void;
  readonly readOnly: boolean;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: stage.value });
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const lanePlan = crmPipelineLaneQuery(
    query,
    stageField.key,
    stageValues?.length ? stageValues : [stage.value],
  );
  const lane = useCrmTableQuery(
    lanePlan.kind === "query"
      ? { ...lanePlan.query, cursor: cursors.at(-1) }
      : undefined,
    ownerId,
    { enabled: lanePlan.kind === "query" },
  );
  const titleField =
    lanePlan.kind === "query"
      ? lane.data?.object.fields.find(
          (field) => field.type === "text" && field.required,
        )
      : undefined;
  const rows =
    lanePlan.kind === "query"
      ? applyOptimisticPipelineRows(
          lane.data?.rows ?? [],
          stage.value,
          moves,
          (move) =>
            crmPipelineStageMatches(query.filters, stageField.key, move.to),
        )
      : [];
  const isPending = lanePlan.kind === "query" && lane.isPending;
  const isError =
    lanePlan.kind === "invalid" || (lanePlan.kind === "query" && lane.isError);
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
          {count === undefined ? "Unavailable" : count.toLocaleString()}
        </span>
      </header>
      <div className="min-h-24 space-y-2 p-2">
        {isPending ? (
          <p role="status" className="p-2 text-11 text-text-tertiary">
            Loading...
          </p>
        ) : null}
        {isError ? (
          <div role="alert" className="p-2 text-11 text-destructive">
            Could not load this lane.
            <Button
              size="sm"
              variant="outline"
              onClick={() => void lane.refetch()}
            >
              Retry lane
            </Button>
          </div>
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
        {!isPending && !isError && rows.length === 0 ? (
          <p className="p-2 text-center text-11 text-text-tertiary">
            No records
          </p>
        ) : null}
        <div className="flex justify-between gap-2">
          {cursors.length > 1 ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={lane.isFetching}
              onClick={() => setCursors((current) => current.slice(0, -1))}
            >
              Previous cards
            </Button>
          ) : null}
          {lane.data?.next_cursor ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={lane.isFetching}
              onClick={() =>
                setCursors((current) => [
                  ...current,
                  lane.data?.next_cursor ?? undefined,
                ])
              }
            >
              Next cards
            </Button>
          ) : null}
        </div>
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
    stageField ? crmPipelineCountQuery(query, stageField.key) : undefined,
    ownerId,
    Boolean(stageField),
  );
  const move = useMoveCrmRecord(ownerId);
  const client = useQueryClient();
  const requests = useRef(
    new Map<
      string,
      {
        id: string;
        expectedRevision: number;
        to: string;
        idempotencyKey: string;
      }
    >(),
  );
  const [uncertain, setUncertain] = useState<string[]>([]);
  const [moves, setMoves] = useState<Record<string, CrmOptimisticPipelineMove>>(
    {},
  );
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor),
  );

  async function moveRecord(record: CrmRecordRow, from: string, to: string) {
    if (from === to || moves[record.id]) return;
    const previous = requests.current.get(record.id);
    if (previous && previous.to !== to) {
      toast.error("Resolve the previous move before choosing another stage.");
      return;
    }
    const request = previous ?? {
      id: record.id,
      expectedRevision: record.revision,
      to,
      idempotencyKey: crypto.randomUUID(),
    };
    requests.current.set(record.id, request);
    setMoves((current) => ({ ...current, [record.id]: { record, from, to } }));
    try {
      await move.mutateAsync(request);
      requests.current.delete(record.id);
      setUncertain((ids) => ids.filter((id) => id !== record.id));
    } catch (error) {
      if (
        error instanceof ApiError &&
        [400, 403, 404, 409, 422].includes(error.status)
      ) {
        requests.current.delete(record.id);
      } else setUncertain((ids) => [...new Set([...ids, record.id])]);
      toast.error("Could not confirm the move", {
        description:
          error instanceof Error
            ? error.message
            : "Refresh and retry the same move.",
      });
    } finally {
      void client.invalidateQueries({
        queryKey: ["crm", ownerId ?? object.data?.owner_id],
      });
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
  const boardQuery = {
    ...query,
    select: query.select
      ? [
          ...new Set([
            ...query.select,
            stageField.key,
            ...(object.data?.fields
              .filter((field) => field.type === "text" && field.required)
              .map((field) => field.key) ?? []),
          ]),
        ]
      : undefined,
  };
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragEnd={dragEnd}
    >
      <div className="space-y-3">
        {counts.isPending ? (
          <p role="status" className="text-12">
            Loading pipeline counts…
          </p>
        ) : null}
        {counts.isError ? (
          <p role="alert" className="text-12 text-destructive">
            Pipeline counts are unavailable.{" "}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void counts.refetch()}
            >
              Retry counts
            </Button>
          </p>
        ) : null}
        {uncertain.map((id) => (
          <div key={id} role="alert" className="text-12">
            A move has an uncertain result. Records are refreshing.{" "}
            <Button
              type="button"
              variant="outline"
              disabled={move.isPending}
              onClick={async () => {
                const request = requests.current.get(id);
                if (!request) return;
                try {
                  await move.mutateAsync(request);
                  requests.current.delete(id);
                  setUncertain((ids) => ids.filter((entry) => entry !== id));
                } catch (error) {
                  if (
                    error instanceof ApiError &&
                    [400, 403, 404, 409, 422].includes(error.status)
                  ) {
                    requests.current.delete(id);
                    setUncertain((ids) => ids.filter((entry) => entry !== id));
                  }
                  toast.error("Could not confirm the move", {
                    description:
                      error instanceof Error
                        ? error.message
                        : "Retry when connected.",
                  });
                } finally {
                  void client.invalidateQueries({
                    queryKey: ["crm", ownerId ?? object.data?.owner_id],
                  });
                }
              }}
            >
              Retry same move
            </Button>
          </div>
        ))}
        <div className="flex gap-3 overflow-x-auto pb-2">
          {liveStages.map((stage) => (
            <PipelineLane
              key={`${ownerId}:${JSON.stringify(boardQuery)}:${stage.value}`}
              query={boardQuery}
              stageField={stageField}
              stage={stage}
              count={
                counts.isError || counts.isPending
                  ? undefined
                  : (countByStage.get(stage.value) ?? 0)
              }
              ownerId={ownerId}
              moves={moves}
              onMove={(record, from, to) => void moveRecord(record, from, to)}
              onSelectRecord={onSelectRecord}
              readOnly={readOnly}
            />
          ))}
          {archivedStages.length > 0 || archivedCount > 0 ? (
            <PipelineLane
              key={`${ownerId}:${JSON.stringify(boardQuery)}:archived`}
              query={boardQuery}
              stageField={stageField}
              stage={{ value: "__archived_stages__", label: "Archived stages" }}
              stageValues={archivedStages.map((stage) => stage.value)}
              count={
                counts.isError || counts.isPending ? undefined : archivedCount
              }
              ownerId={ownerId}
              moves={moves}
              onMove={(record, from, to) => void moveRecord(record, from, to)}
              onSelectRecord={onSelectRecord}
              readOnly
            />
          ) : null}
        </div>
      </div>
    </DndContext>
  );
}

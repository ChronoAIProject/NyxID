/* eslint-disable react-refresh/only-export-components -- the closed catalog intentionally owns its React implementations. */
import type { ReactNode } from "react";
import { z } from "zod";
import {
  Bar,
  BarChart as RechartsBarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart as RechartsPieChart,
  XAxis,
  YAxis,
} from "recharts";
import type { BaseComponentProps } from "@json-render/react";
import { CrmObjectTable } from "@/components/crm/object-table";
import { CrmPipelineBoard } from "@/components/crm/pipeline-board";
import { CrmRecordForm } from "@/components/crm/record-form";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { cn } from "@/lib/utils";
import {
  aggregateResultToChartPoints,
  formatAggregateValue,
} from "@/lib/crm/views/chart-adapters";
import {
  crmFieldKeySchema,
  crmViewIdentifierSchema,
  type CrmAggregateResult,
} from "@/lib/crm/views/schema";
import {
  useCrmViewOwnerId,
  useInvalidateCrmViewQueries,
  useCrmNamedAggregateQuery,
  useCrmNamedTableQuery,
} from "@/lib/crm/views/query-runner";
import { useCrmObject } from "@/hooks/use-crm";
import { useCrmRecordNavigation } from "@/lib/crm/record-navigation";

export type CrmViewQueryRequirement =
  | { readonly kind: "none" }
  | { readonly kind: "table" }
  | { readonly kind: "aggregate"; readonly grouped: boolean };

interface CrmViewCatalogEntry<PropsSchema extends z.ZodType> {
  readonly props: PropsSchema;
  readonly description: string;
  readonly slots: readonly string[];
  readonly query: CrmViewQueryRequirement;
  readonly allowsChildren: boolean;
  readonly component: (
    context: BaseComponentProps<z.infer<PropsSchema>>,
  ) => ReactNode;
}

const stackPropsSchema = z
  .object({
    gap: z.enum(["sm", "md", "lg"]),
    direction: z.enum(["vertical", "horizontal"]).optional(),
  })
  .strict();

const gridPropsSchema = z
  .object({ columns: z.number().int().min(1).max(4) })
  .strict();

const textPropsSchema = z
  .object({
    content: z.string().min(1).max(2_000),
    variant: z.enum(["title", "heading", "body", "muted"]),
  })
  .strict();

const metricPropsSchema = z
  .object({ queryId: crmViewIdentifierSchema, title: z.string().min(1) })
  .strict();

const recordFormPropsSchema = z
  .object({
    object: crmFieldKeySchema,
    fields: z.array(crmFieldKeySchema).optional(),
    title: z.string().min(1),
    submitLabel: z.string().min(1).optional(),
  })
  .strict();

const chartPropsSchema = z
  .object({
    queryId: crmViewIdentifierSchema,
    title: z.string().min(1),
    height: z.enum(["small", "medium", "large"]).optional(),
  })
  .strict();

const objectTablePropsSchema = z
  .object({
    queryId: crmViewIdentifierSchema,
    columns: z.array(crmFieldKeySchema).optional(),
  })
  .strict();

const pipelineBoardPropsSchema = z
  .object({ queryId: crmViewIdentifierSchema })
  .strict();

const gapClasses = {
  sm: "gap-2",
  md: "gap-4",
  lg: "gap-6",
} as const;

function StackComponent({
  props,
  children,
}: BaseComponentProps<z.infer<typeof stackPropsSchema>>) {
  return (
    <div
      className={cn(
        "flex min-w-0",
        gapClasses[props.gap],
        props.direction === "horizontal"
          ? "flex-col md:flex-row md:items-start"
          : "flex-col",
      )}
    >
      {children}
    </div>
  );
}

const gridClasses = {
  1: "grid-cols-1",
  2: "grid-cols-1 md:grid-cols-2",
  3: "grid-cols-1 md:grid-cols-2 xl:grid-cols-3",
  4: "grid-cols-1 md:grid-cols-2 xl:grid-cols-4",
} as const;

function GridComponent({
  props,
  children,
}: BaseComponentProps<z.infer<typeof gridPropsSchema>>) {
  return (
    <div
      className={cn(
        "grid min-w-0 gap-4",
        gridClasses[props.columns as keyof typeof gridClasses],
      )}
    >
      {children}
    </div>
  );
}

const textClasses = {
  title: "type-compact-page-title sm:type-page-title",
  heading: "type-card-title",
  body: "type-secondary text-foreground",
  muted: "type-meta text-muted-foreground",
} as const;

function TextComponent({
  props,
}: BaseComponentProps<z.infer<typeof textPropsSchema>>) {
  const Tag = props.variant === "title" ? "h2" : "p";
  return <Tag className={textClasses[props.variant]}>{props.content}</Tag>;
}

function ComponentQueryError({ message }: { readonly message: string }) {
  return (
    <div
      role="alert"
      className="rounded-lg border border-destructive/20 bg-destructive/[0.04] px-4 py-3 text-12 text-destructive"
    >
      {message}
    </div>
  );
}

function MetricComponent({
  props,
}: BaseComponentProps<z.infer<typeof metricPropsSchema>>) {
  const { namedQuery, result } = useCrmNamedAggregateQuery(props.queryId);
  if (!namedQuery) {
    return <ComponentQueryError message="This metric query is unavailable." />;
  }
  if (result.isPending) {
    return (
      <div role="status" className="rounded-xl border border-border/50 p-4">
        <p className="type-meta text-text-tertiary">Loading {props.title}...</p>
      </div>
    );
  }
  if (result.isError || !result.data) {
    return <ComponentQueryError message={`Could not load ${props.title}.`} />;
  }

  return (
    <section className="rounded-xl border border-border/50 bg-card p-4 shadow-sm">
      <p className="type-table-header uppercase tracking-[1.5px] text-text-tertiary">
        {props.title}
      </p>
      <p className="mt-2 type-metric tabular-nums text-foreground">
        {formatAggregateValue(result.data.total, result.data.unit)}
      </p>
    </section>
  );
}

function RecordFormComponent({
  props,
}: BaseComponentProps<z.infer<typeof recordFormPropsSchema>>) {
  const ownerId = useCrmViewOwnerId();
  const object = useCrmObject(props.object, ownerId);
  const invalidateQueries = useInvalidateCrmViewQueries();
  if (object.isPending) {
    return (
      <p role="status" className="text-12 text-text-tertiary">
        Loading {props.title}...
      </p>
    );
  }
  if (object.isError || !object.data) {
    return <ComponentQueryError message={`Could not load ${props.title}.`} />;
  }
  if (object.data.access === "read") {
    return null;
  }
  return (
    <CrmRecordForm
      ownerId={ownerId}
      object={object.data}
      fields={props.fields}
      title={props.title}
      submitLabel={props.submitLabel}
      onCreated={() => void invalidateQueries()}
    />
  );
}

const chartConfig = {
  value: { label: "Value", color: "var(--color-primary)" },
} satisfies ChartConfig;

const chartHeightClasses = {
  small: "h-40",
  medium: "h-56",
  large: "h-72",
} as const;

const pieColors = [
  "var(--color-primary)",
  "var(--color-success)",
  "var(--color-info)",
  "var(--color-warning)",
  "var(--color-muted-foreground)",
] as const;

interface AggregateChartFrameProps {
  readonly title: string;
  readonly height: "small" | "medium" | "large";
  readonly result: CrmAggregateResult;
  readonly children: (
    points: ReturnType<typeof aggregateResultToChartPoints>,
    unit: CrmAggregateResult["unit"],
  ) => ReactNode;
}

function AggregateChartFrame({
  title,
  height,
  result,
  children,
}: AggregateChartFrameProps) {
  const points = aggregateResultToChartPoints(result);
  return (
    <section className="min-w-0 rounded-xl border border-border/50 bg-card p-4 shadow-sm">
      <h3 className="mb-3 type-card-title">{title}</h3>
      {points.length === 0 ? (
        <div className="flex h-40 items-center justify-center rounded-lg border border-dashed border-border text-12 text-text-tertiary">
          No data matches this view.
        </div>
      ) : (
        <>
          <ChartContainer
            config={chartConfig}
            className={cn("aspect-auto w-full", chartHeightClasses[height])}
          >
            {children(points, result.unit)}
          </ChartContainer>
          <dl className="sr-only" aria-label={`${title} values`}>
            {points.map((point) => (
              <div key={point.key}>
                <dt>{point.label}</dt>
                <dd>{formatAggregateValue(point.value, result.unit)}</dd>
              </div>
            ))}
          </dl>
          {result.truncated ? (
            <p className="mt-2 type-meta text-text-tertiary">
              Additional groups are combined into Other.
            </p>
          ) : null}
        </>
      )}
    </section>
  );
}

function AggregateChartState({
  queryId,
  title,
  height,
  render,
  pie = false,
}: {
  readonly queryId: string;
  readonly title: string;
  readonly height?: "small" | "medium" | "large";
  readonly render: AggregateChartFrameProps["children"];
  readonly pie?: boolean;
}) {
  const { namedQuery, result } = useCrmNamedAggregateQuery(queryId);
  if (!namedQuery) {
    return <ComponentQueryError message="This chart query is unavailable." />;
  }
  if (result.isPending) {
    return (
      <div
        role="status"
        className={cn(
          "flex items-center justify-center rounded-xl border border-border/50 text-12 text-text-tertiary",
          chartHeightClasses[height ?? "medium"],
        )}
      >
        Loading {title}...
      </div>
    );
  }
  if (result.isError || !result.data) {
    return <ComponentQueryError message={`Could not load ${title}.`} />;
  }
  if (
    pie &&
    (!["count", "sum"].includes(namedQuery.query.measure.op) ||
      (result.data.total !== null && result.data.total < 0) ||
      result.data.points.some(
        (point) => point.value !== null && point.value < 0,
      ))
  ) {
    return (
      <ComponentQueryError message="Pie charts require nonnegative count or sum values that form parts of a whole." />
    );
  }
  return (
    <AggregateChartFrame
      title={title}
      height={height ?? "medium"}
      result={result.data}
    >
      {render}
    </AggregateChartFrame>
  );
}

function BarChartComponent({
  props,
}: BaseComponentProps<z.infer<typeof chartPropsSchema>>) {
  return (
    <AggregateChartState
      queryId={props.queryId}
      title={props.title}
      height={props.height}
      render={(points, unit) => (
        <RechartsBarChart data={points} accessibilityLayer>
          <CartesianGrid vertical={false} strokeDasharray="3 3" />
          <XAxis
            dataKey="label"
            tickLine={false}
            axisLine={false}
            tickMargin={8}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            width={88}
            tickFormatter={(value: number) => formatAggregateValue(value, unit)}
          />
          <ChartTooltip
            cursor={{ fill: "var(--color-muted)", opacity: 0.4 }}
            content={
              <ChartTooltipContent
                hideLabel
                formatter={(value) => formatAggregateValue(Number(value), unit)}
              />
            }
          />
          <Bar dataKey="value" fill="var(--color-value)" radius={4} />
        </RechartsBarChart>
      )}
    />
  );
}

function PieChartComponent({
  props,
}: BaseComponentProps<z.infer<typeof chartPropsSchema>>) {
  return (
    <AggregateChartState
      queryId={props.queryId}
      title={props.title}
      height={props.height}
      pie
      render={(points, unit) => (
        <RechartsPieChart accessibilityLayer>
          <ChartTooltip
            content={
              <ChartTooltipContent
                nameKey="label"
                formatter={(value) => formatAggregateValue(Number(value), unit)}
              />
            }
          />
          <Pie
            data={points}
            dataKey="value"
            nameKey="label"
            innerRadius="42%"
            outerRadius="78%"
            paddingAngle={2}
          >
            {points.map((point, index) => (
              <Cell
                key={point.key}
                fill={pieColors[index % pieColors.length]}
              />
            ))}
          </Pie>
        </RechartsPieChart>
      )}
    />
  );
}

function ObjectTableComponent({
  props,
}: BaseComponentProps<z.infer<typeof objectTablePropsSchema>>) {
  const { namedQuery, ownerId } = useCrmNamedTableQuery(props.queryId);
  const onSelectRecord = useCrmRecordNavigation();
  if (!namedQuery) {
    return <ComponentQueryError message="This table query is unavailable." />;
  }
  return (
    <CrmObjectTable
      query={namedQuery.query}
      columns={props.columns}
      ownerId={ownerId}
      onSelectRecord={onSelectRecord}
    />
  );
}

function PipelineBoardComponent({
  props,
}: BaseComponentProps<z.infer<typeof pipelineBoardPropsSchema>>) {
  const { namedQuery, ownerId } = useCrmNamedTableQuery(props.queryId);
  const onSelectRecord = useCrmRecordNavigation();
  if (!namedQuery) {
    return (
      <ComponentQueryError message="This pipeline query is unavailable." />
    );
  }
  return (
    <CrmPipelineBoard
      query={namedQuery.query}
      ownerId={ownerId}
      onSelectRecord={onSelectRecord}
    />
  );
}

function defineCatalogEntry<PropsSchema extends z.ZodType>(
  entry: CrmViewCatalogEntry<PropsSchema>,
) {
  return entry;
}

export const crmViewComponentCatalog = {
  Stack: defineCatalogEntry({
    props: stackPropsSchema,
    description: "A vertical or responsive horizontal layout stack",
    slots: ["default"],
    query: { kind: "none" },
    allowsChildren: true,
    component: StackComponent,
  }),
  Grid: defineCatalogEntry({
    props: gridPropsSchema,
    description: "A responsive one-to-four column grid",
    slots: ["default"],
    query: { kind: "none" },
    allowsChildren: true,
    component: GridComponent,
  }),
  Text: defineCatalogEntry({
    props: textPropsSchema,
    description: "Literal saved-view text",
    slots: [],
    query: { kind: "none" },
    allowsChildren: false,
    component: TextComponent,
  }),
  Metric: defineCatalogEntry({
    props: metricPropsSchema,
    description: "A single aggregate value",
    slots: [],
    query: { kind: "aggregate", grouped: false },
    allowsChildren: false,
    component: MetricComponent,
  }),
  RecordForm: defineCatalogEntry({
    props: recordFormPropsSchema,
    description: "A CRM record creation form",
    slots: [],
    query: { kind: "none" },
    allowsChildren: false,
    component: RecordFormComponent,
  }),
  BarChart: defineCatalogEntry({
    props: chartPropsSchema,
    description: "A bar chart backed by a grouped aggregate query",
    slots: [],
    query: { kind: "aggregate", grouped: true },
    allowsChildren: false,
    component: BarChartComponent,
  }),
  PieChart: defineCatalogEntry({
    props: chartPropsSchema,
    description: "A pie chart backed by a grouped aggregate query",
    slots: [],
    query: { kind: "aggregate", grouped: true },
    allowsChildren: false,
    component: PieChartComponent,
  }),
  ObjectTable: defineCatalogEntry({
    props: objectTablePropsSchema,
    description: "A CRM record table",
    slots: [],
    query: { kind: "table" },
    allowsChildren: false,
    component: ObjectTableComponent,
  }),
  PipelineBoard: defineCatalogEntry({
    props: pipelineBoardPropsSchema,
    description: "A CRM pipeline board",
    slots: [],
    query: { kind: "table" },
    allowsChildren: false,
    component: PipelineBoardComponent,
  }),
} as const;

export type CrmViewComponentType = keyof typeof crmViewComponentCatalog;

export function isCrmViewComponentType(
  type: string,
): type is CrmViewComponentType {
  return Object.hasOwn(crmViewComponentCatalog, type);
}

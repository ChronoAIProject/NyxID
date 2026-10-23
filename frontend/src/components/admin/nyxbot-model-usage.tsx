import { useState, type ReactNode } from "react";
import {
  useNyxbotModelUsage,
  type ModelFilter,
} from "@/hooks/use-nyxbot-model-usage";
import type { NyxbotUsageModel } from "@/schemas/nyxbot-model-usage";
import {
  accountAnalytics,
  modelCost,
  serviceAnalytics,
  serviceLabel,
} from "@/lib/nyxbot-model-usage";
import {
  BREAKDOWN_LABELS,
  MEASURE_LABELS,
  newPanel,
} from "@/lib/usage-analytics";
import { ChartView } from "@/components/billing-analytics/chart-view";
import { ChevronDown } from "lucide-react";
import { ModelTrends } from "./nyxbot-model-trends";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
const count = (n: number) => n.toLocaleString();
// The Usage dashboard's Top billing accounts panel, rendered from NyxBot data.
const ACCOUNTS_PANEL = newPanel({
  title: "Top billing accounts",
  breakdown: "owner",
  chart: "bar",
  top: 10,
});
const SERVICES_PANEL = newPanel({
  title: "Model services",
  measure: "requests",
  breakdown: "service",
  chart: "bar",
  top: 20,
});
function Table({
  title,
  columns,
  children,
}: {
  title: string;
  columns: string[];
  children: ReactNode;
}) {
  return (
    <div
      className="min-w-0 overflow-x-auto rounded-xl border border-border/50 bg-card"
      role="region"
      aria-label={title}
      tabIndex={0}
    >
      <table className="w-full text-12">
        <caption className="sr-only">{title}</caption>
        <thead>
          <tr>
            {columns.map((column) => (
              <th
                key={column}
                scope="col"
                className="h-8 whitespace-nowrap border-b border-border px-3 text-left text-10 font-semibold uppercase tracking-wide text-text-tertiary"
              >
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}
function Cell({
  children,
  heading = false,
}: {
  children: ReactNode;
  heading?: boolean;
}) {
  const Tag = heading ? "th" : "td";
  return (
    <Tag
      scope={heading ? "row" : undefined}
      className={`border-b border-border/50 px-3 py-2.5 text-left tabular-nums ${heading ? "min-w-44 font-semibold" : "whitespace-nowrap font-mono text-11"}`}
    >
      {children}
    </Tag>
  );
}
function ModelName({
  row,
  onSelect,
}: {
  row: NyxbotUsageModel;
  onSelect: (filter: ModelFilter) => void;
}) {
  if (row.is_other) return <span>Other · {count(row.groups)} models</span>;
  return (
    <button
      type="button"
      className="text-left underline decoration-border underline-offset-4 hover:decoration-foreground focus-visible:outline-2 focus-visible:outline-offset-4"
      onClick={() =>
        onSelect(row.model === null ? { unknown: true } : { model: row.model })
      }
    >
      {row.model ?? "Unknown model"}
    </button>
  );
}
const tokenFields = [
  "requests",
  "prompt_tokens",
  "completion_tokens",
  "cached_tokens",
  "cache_creation_tokens",
  "total_tokens",
] as const;
const tokenLabels = [
  "Requests",
  "Input tokens",
  "Output tokens",
  "Cache read",
  "Cache write",
  "Total tokens",
];
export function NyxbotModelUsage({ from, to }: { from: string; to: string }) {
  const [filter, setFilter] = useState<ModelFilter>({});
  const [draft, setDraft] = useState("");
  const query = useNyxbotModelUsage(from, to, filter, Boolean(from && to));
  const data = query.data;
  const changeFilter = (value: ModelFilter) => {
    setFilter(value);
    setDraft(value.model ?? "");
  };
  const partial = (data?.totals.unknown_cost_events ?? 0) > 0;
  const candidates = data?.excluded.find(
    (e) => e.population === "unattributable_legacy",
  );
  const audioImage = data?.models.some(
    (m) =>
      m.usage.audio_input_tokens ||
      m.usage.audio_output_tokens ||
      m.usage.image_input_tokens ||
      m.usage.image_output_tokens,
  );
  return (
    <section className="space-y-4" aria-label="NyxBot model usage">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="font-display text-15 font-semibold">
            Cost and billing by model
          </h2>
          <p className="mt-1 max-w-3xl text-12 text-muted-foreground">
            Model executions by NyxBot and specialists. Credits include wallet,
            grant and allowance funding. Model names are recorded billing
            identities and may be request aliases.
          </p>
        </div>
        <Button
          variant="outline"
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
        >
          Refresh model usage
        </Button>
      </div>
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (draft.trim()) changeFilter({ model: draft.trim() });
        }}
      >
        <label className="min-w-0 space-y-1 text-11 text-muted-foreground">
          Billing model
          <Input
            value={draft}
            maxLength={256}
            placeholder="Exact model name"
            onChange={(e) => setDraft(e.target.value)}
          />
        </label>
        <Button type="submit" variant="outline" disabled={!draft.trim()}>
          Apply model
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => changeFilter({ unknown: true })}
        >
          Unknown model
        </Button>
        {(filter.model || filter.unknown) && (
          <Button
            type="button"
            variant="outline"
            onClick={() => changeFilter({})}
          >
            All models
          </Button>
        )}
        <p role="status" className="pb-2 text-11 text-text-tertiary">
          {filter.unknown
            ? "Unknown model selected"
            : filter.model
              ? `Model: ${filter.model}`
              : "All models"}
          {query.isFetching ? " · Updating…" : ""}
        </p>
      </form>
      {query.isError && (
        <div role="alert">
          <ErrorBanner
            message="Unable to load NyxBot model usage."
            onRetry={() => void query.refetch()}
          />
        </div>
      )}
      {query.isPending && (
        <div role="status" aria-label="Loading model usage">
          <Skeleton className="h-40 w-full" />
        </div>
      )}
      {data && (
        <>
          {data.excluded.find(
            (e) => e.population === "unclassified" && e.usage.events > 0,
          ) && (
            <p
              className="rounded-lg border border-warning/40 p-3 text-12"
              role="note"
            >
              Some confirmed historical assistant activity lacks evidence of
              model execution. Its requests and costs are excluded from the
              charts and tables; see Scope and data freshness for amounts.
            </p>
          )}
          {candidates && candidates.usage.events > 0 && (
            <p
              className="rounded-lg border border-warning/40 p-3 text-12"
              role="note"
            >
              Some historical usage cannot be attributed.{" "}
              {count(candidates.usage.events)} historical platform events are
              excluded; their NyxBot share is unknown.
            </p>
          )}
          {partial && (
            <p role="note" className="text-12 text-warning">
              Partial cost estimate · {count(data.totals.unknown_cost_events)}{" "}
              billable events have unavailable historical prices. ≥ shows the
              known subtotal; Unavailable means no cost could be priced.
            </p>
          )}
          {data.totals.events === 0 ? (
            <p className="rounded-lg border border-dashed border-border p-5 text-12 text-muted-foreground">
              No attributed model usage in this period
              {filter.model || filter.unknown ? " for this model" : ""}.
            </p>
          ) : (
            <>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
                {(
                  [
                    ["Gross cost", "gross_cost"],
                    ["Wallet funded", "wallet_cost"],
                    ["Grant funded", "grant_cost"],
                    ["Allowance funded", "allowance_cost"],
                  ] as const
                ).map(([label, field]) => (
                  <Card key={field} className="min-w-0 p-4">
                    <p className="text-10 uppercase tracking-wide text-muted-foreground">
                      {label}
                    </p>
                    <p className="mt-2 break-words font-display text-22 tabular-nums">
                      {modelCost(data.totals, field)}
                    </p>
                    <p className="mt-1 text-10 text-text-tertiary">credits</p>
                  </Card>
                ))}
              </div>
              <ModelTrends data={data} kind="cost" />
              <Table
                title="Cost and billing by model in exact credits"
                columns={[
                  "Model",
                  "Gross credits",
                  "Wallet credits",
                  "Grant credits",
                  "Allowance credits",
                  "Unpriced events",
                ]}
              >
                {data.models.map((row, index) => (
                  <tr key={index}>
                    <Cell heading>
                      <ModelName row={row} onSelect={changeFilter} />
                    </Cell>
                    {(
                      [
                        "gross_cost",
                        "wallet_cost",
                        "grant_cost",
                        "allowance_cost",
                      ] as const
                    ).map((field) => (
                      <Cell key={field}>{modelCost(row.usage, field)}</Cell>
                    ))}
                    <Cell>{count(row.usage.unknown_cost_events)}</Cell>
                  </tr>
                ))}
              </Table>
              <div>
                <h2 className="font-display text-15 font-semibold">
                  Traffic by model
                </h2>
                <p className="mt-1 text-12 text-muted-foreground">
                  Provider-reported token counts. Total = input + output. Cache
                  counts can overlap input; audio and image counts are subsets.
                </p>
              </div>
              <ModelTrends data={data} kind="traffic" />
              <Table
                title="Traffic by model with exact token counts"
                columns={[
                  "Model",
                  ...tokenLabels,
                  ...(audioImage
                    ? [
                        "Audio input",
                        "Audio output",
                        "Image input",
                        "Image output",
                      ]
                    : []),
                ]}
              >
                {data.models.map((row, index) => (
                  <tr key={index}>
                    <Cell heading>
                      <ModelName row={row} onSelect={changeFilter} />
                    </Cell>
                    {tokenFields.map((field) => (
                      <Cell key={field}>{count(row.usage[field])}</Cell>
                    ))}
                    {audioImage &&
                      (
                        [
                          "audio_input_tokens",
                          "audio_output_tokens",
                          "image_input_tokens",
                          "image_output_tokens",
                        ] as const
                      ).map((field) => (
                        <Cell key={field}>{count(row.usage[field])}</Cell>
                      ))}
                  </tr>
                ))}
              </Table>
              <Card
                role="article"
                aria-label="Model services"
                className="min-w-0 p-4"
              >
                <div className="mb-4">
                  <h2 className="font-display text-15 font-semibold leading-none tracking-tight">
                    Model services
                  </h2>
                  <p className="mt-1 text-12 text-muted-foreground">
                    {MEASURE_LABELS.requests} · {BREAKDOWN_LABELS.service}
                  </p>
                </div>
                <ChartView
                  data={serviceAnalytics(data)}
                  panel={SERVICES_PANEL}
                />
                <details className="group mt-4 border-t border-border/50 pt-3">
                  <summary className="flex cursor-pointer list-none items-center justify-between text-11 font-medium text-muted-foreground [&::-webkit-details-marker]:hidden">
                    Cost and tokens by service
                    <ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />
                  </summary>
                  <p className="mt-2 text-11 text-muted-foreground">
                    The inference service is recorded on every model execution,
                    including usage from before model names were captured.
                  </p>
                  <div className="mt-3">
                    <Table
                      title="Model services usage"
                      columns={["Service", "Gross credits", ...tokenLabels]}
                    >
                      {data.services.map((row, index) => (
                        <tr key={index}>
                          <Cell heading>
                            {row.is_other
                              ? `Other · ${count(row.groups)} services`
                              : serviceLabel(row)}
                          </Cell>
                          <Cell>{modelCost(row.usage, "gross_cost")}</Cell>
                          {tokenFields.map((field) => (
                            <Cell key={field}>{count(row.usage[field])}</Cell>
                          ))}
                        </tr>
                      ))}
                    </Table>
                  </div>
                </details>
              </Card>
              <Card
                role="article"
                aria-label="Top billing accounts"
                className="min-w-0 p-4"
              >
                <div className="mb-4">
                  <h2 className="font-display text-15 font-semibold leading-none tracking-tight">
                    Top billing accounts
                  </h2>
                  <p className="mt-1 text-12 text-muted-foreground">
                    {MEASURE_LABELS.cost} · {BREAKDOWN_LABELS.owner}
                  </p>
                </div>
                <ChartView
                  data={accountAnalytics(data)}
                  panel={ACCOUNTS_PANEL}
                />
                <details className="group mt-4 border-t border-border/50 pt-3">
                  <summary className="flex cursor-pointer list-none items-center justify-between text-11 font-medium text-muted-foreground [&::-webkit-details-marker]:hidden">
                    Models by billing account
                    <ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />
                  </summary>
                  <p className="mt-2 text-11 text-muted-foreground">
                    Ranked by known gross credits, then requests. Missing prices
                    can affect cost ranking. Each account shows up to 20 models
                    and a summed Other row.
                  </p>
                  <div className="mt-3 space-y-4">
                    {data.accounts.map(({ account, usage, models }) => (
                      <div key={account.id} className="min-w-0 space-y-2">
                        <div className="flex flex-wrap justify-between gap-2">
                          <div>
                            <h3 className="text-13 font-semibold">
                              {account.display_name}
                            </h3>
                            <p className="text-11 text-text-tertiary">
                              {account.user_type === "org"
                                ? "Organization"
                                : (account.email ?? "Billing account")}
                            </p>
                          </div>
                          <p className="text-12 tabular-nums">
                            {modelCost(usage, "gross_cost")} credits ·{" "}
                            {count(usage.requests)} requests ·{" "}
                            {count(usage.total_tokens)} tokens
                          </p>
                        </div>
                        <Table
                          title={`${account.display_name} usage by model`}
                          columns={["Model", "Gross credits", ...tokenLabels]}
                        >
                          {models.map((row, index) => (
                            <tr key={index}>
                              <Cell heading>
                                <ModelName row={row} onSelect={changeFilter} />
                              </Cell>
                              <Cell>{modelCost(row.usage, "gross_cost")}</Cell>
                              {tokenFields.map((field) => (
                                <Cell key={field}>
                                  {count(row.usage[field])}
                                </Cell>
                              ))}
                            </tr>
                          ))}
                        </Table>
                      </div>
                    ))}
                    {data.other_accounts && (
                      <p className="text-12 tabular-nums">
                        <strong>
                          Other · {count(data.other_accounts.groups)} billing
                          accounts
                        </strong>{" "}
                        · {modelCost(data.other_accounts.usage, "gross_cost")}{" "}
                        credits · {count(data.other_accounts.usage.requests)}{" "}
                        requests ·{" "}
                        {count(data.other_accounts.usage.total_tokens)} tokens
                      </p>
                    )}
                  </div>
                </details>
              </Card>
            </>
          )}
          <details className="text-11 text-muted-foreground">
            <summary className="cursor-pointer">
              Scope and data freshness
            </summary>
            <p className="mt-2">
              Orchestration requests to llm-nyx and ordinary tool calls are
              excluded from model totals. They can have separate charges. Legacy
              free summaries lack attribution. Retained raw records may permit a
              separate recovery; this report does not scan their full history.
              Historical attribution also needs retained assistant credential,
              rotation, conversation, or provisioning evidence. Unknown model
              means an execution had no recorded billing model; it does not use
              a conversation’s selected model. Proxied executions record the
              provider-reported model; earlier rows remain unknown.
            </p>
            {data.excluded
              .filter((e) => e.population !== "unattributable_legacy")
              .map((e) => (
                <p key={e.population}>
                  {e.population === "tool"
                    ? "Ordinary tools"
                    : e.population === "unclassified"
                      ? "Historical assistant activity without execution evidence"
                      : "Orchestration"}
                  : {count(e.usage.requests)} requests ·{" "}
                  {modelCost(e.usage, "gross_cost")} credits excluded.
                </p>
              ))}
            <p className="mt-2">
              {data.freshness.validated
                ? "Validated totals"
                : "Updating totals"}{" "}
              · includes recent activity · summaries updated through{" "}
              {new Date(data.freshness.rolled_up_through).toLocaleString()}.
            </p>
          </details>
        </>
      )}
    </section>
  );
}

import { useId, useState } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Area,
  AreaChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useAssistantAnalytics } from "@/hooks/use-assistant-analytics";
import { useAssistantFeedbackAdmin } from "@/hooks/use-assistant-feedback-admin";
import {
  nyxbotDailyData,
  nyxbotDateRange,
  nyxbotWindow,
} from "@/lib/assistant-analytics";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AssistantFeedbackPanel } from "./assistant-feedback-panel";
import { ChartGradients } from "@/components/billing-analytics/chart-gradients";
import {
  chartTick,
  chartTooltipStyle,
  chartTooltipTextStyle,
} from "@/components/billing-analytics/chart-style";
import { FilterCard } from "@/components/billing-analytics/filter-picker";
import "@/components/billing-analytics/visualization.css";
import "@/components/billing-analytics/operations.css";

const number = (value: number) => value.toLocaleString();
const dayLabel = (day: string) =>
  new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
function Stat({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <Card className="min-w-0 px-4 py-4">
      <p className="text-10 font-medium uppercase tracking-[1.5px] text-muted-foreground">
        {label}
      </p>
      <p className="mt-3 font-display text-28 font-medium leading-none tracking-tight tabular-nums">
        {value}
      </p>
      <p className="mt-2 text-10 leading-relaxed text-muted-foreground">
        {detail}
      </p>
    </Card>
  );
}

function Legend({ feedback = false }: { feedback?: boolean }) {
  return (
    <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1.5 text-11 text-foreground">
      <span className="flex items-center gap-1.5">
        <span
          className="size-1.5 rounded-full"
          style={{
            background: feedback
              ? "var(--analytics-series-4)"
              : "var(--analytics-series-1)",
          }}
        />
        {feedback ? "Helpful" : "Active sessions"}
      </span>
      <span className="flex items-center gap-1.5">
        <span
          className="size-1.5 rounded-full"
          style={{
            background: feedback
              ? "var(--analytics-series-5)"
              : "var(--analytics-series-2)",
          }}
        />
        {feedback ? "Not helpful" : "Settled turns"}
      </span>
    </div>
  );
}

export function NyxbotUsage() {
  const chartId = useId().replace(/:/g, "");
  const [dates, setDates] = useState(() => nyxbotDateRange(7));
  const window = nyxbotWindow(dates.from, dates.to);
  const from = window?.from ?? "";
  const to = window?.to ?? "";
  const analytics = useAssistantAnalytics(from, to, Boolean(window));
  // Feedback uses an inclusive upper bound; analytics uses an exclusive one.
  const feedbackParams = new URLSearchParams(
    window ? { from, to: new Date(Date.parse(to) - 1).toISOString() } : {},
  );
  const feedback = useAssistantFeedbackAdmin(feedbackParams, Boolean(window));
  const data = analytics.data;
  const votes = feedback.data?.counts.reduce(
    (total, row) => ({ up: total.up + row.up, down: total.down + row.down }),
    { up: 0, down: 0 },
  );
  const totalVotes = votes ? votes.up + votes.down : 0;
  const daily = window ? nyxbotDailyData(from, to, data, feedback.data) : [];
  const outcomes = data?.outcomes;
  const totalTurns = outcomes
    ? outcomes.completed + outcomes.failed + outcomes.stopped
    : 0;
  return (
    <div className="analytics-visuals space-y-4">
      <div>
        <h2 className="font-display text-15 font-semibold tracking-tight">
          NyxBot activity & feedback
        </h2>
        <p className="mt-1 text-12 text-muted-foreground">
          Understand how NyxBot and specialists are used, where work fails, and
          what users say.
        </p>
      </div>
      <FilterCard
        pickers={
          <div
            className="flex flex-wrap items-end gap-2"
            aria-label="NyxBot date range"
          >
            <Button
              variant="outline"
              onClick={() => setDates(nyxbotDateRange(7))}
            >
              Last 7 days
            </Button>
            <Button
              variant="outline"
              onClick={() => setDates(nyxbotDateRange(30))}
            >
              Last 30 days
            </Button>
            <label className="space-y-1 text-11 text-muted-foreground">
              From
              <Input
                type="date"
                value={dates.from}
                onChange={(event) =>
                  setDates({ ...dates, from: event.target.value })
                }
              />
            </label>
            <label className="space-y-1 text-11 text-muted-foreground">
              To
              <Input
                type="date"
                value={dates.to}
                onChange={(event) =>
                  setDates({ ...dates, to: event.target.value })
                }
              />
            </label>
            <span className="pb-2 text-11 text-text-tertiary">UTC</span>
          </div>
        }
        aside={
          <span className="text-10 text-muted-foreground">Daily · UTC</span>
        }
      />
      {!window ? (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 p-3 text-12 text-destructive"
        >
          Choose a start and end date covering at most 31 days.
        </p>
      ) : (
        <>
          <div
            className="flex flex-wrap items-center gap-2 text-11 text-text-tertiary"
            role="status"
          >
            <span
              className={`size-1.5 rounded-full ${data && !analytics.isError ? "bg-success" : "bg-text-tertiary"}`}
              aria-hidden="true"
            />
            {analytics.isFetching
              ? "Updating activity…"
              : data
                ? `Updated ${new Date(data.generated_at).toLocaleTimeString()} · live count refreshes every 30 seconds`
                : analytics.isPending
                  ? "Loading activity…"
                  : "Activity unavailable"}
          </div>
          {analytics.isError && (
            <div role="alert">
              <ErrorBanner
                message="Unable to load NyxBot activity."
                onRetry={() => void analytics.refetch()}
              />
            </div>
          )}
          {feedback.isError && (
            <div role="alert">
              <ErrorBanner
                message="Unable to load feedback totals."
                onRetry={() => void feedback.refetch()}
              />
            </div>
          )}
          <div className="analytics-summary grid grid-cols-2 lg:grid-cols-4">
            <Stat
              label="Active sessions"
              value={data ? number(data.active_sessions) : "—"}
              detail={
                data
                  ? `Conversations with user input · ${number(data.active_accounts)} accounts`
                  : "Conversations with user input in this period"
              }
            />
            <Stat
              label="Running now"
              value={data ? number(data.running_now) : "—"}
              detail="Live turns with a recent worker heartbeat"
            />
            <Stat
              label="Feedback received"
              value={votes ? number(totalVotes) : "—"}
              detail={
                votes
                  ? `${number(votes.up)} helpful · ${number(votes.down)} not helpful`
                  : "Retained votes submitted in this period"
              }
            />
            <Stat
              label="Helpful feedback"
              value={
                votes && totalVotes
                  ? `${Math.round((votes.up / totalVotes) * 100)}%`
                  : "—"
              }
              detail={
                totalVotes
                  ? `Of ${number(totalVotes)} votes · feedback is optional`
                  : votes
                    ? "No votes in this period"
                    : "Feedback totals are not available yet"
              }
            />
          </div>
          <div className="grid min-w-0 gap-3 lg:grid-cols-2">
            <Card className="analytics-panel analytics-chart min-w-0 p-4">
              <h3 className="font-display text-15 font-semibold leading-none tracking-tight">
                Session activity
              </h3>
              <p className="mb-4 mt-1 text-12 text-muted-foreground">
                Daily conversations with user input and unique settled turns.
              </p>
              <div
                className="analytics-plot h-56"
                aria-label="Daily session activity"
              >
                {!data ? (
                  analytics.isPending ? (
                    <Skeleton className="h-full" />
                  ) : (
                    <p className="pt-16 text-center text-12 text-muted-foreground">
                      Activity is unavailable.
                    </p>
                  )
                ) : !data.active_sessions && !totalTurns ? (
                  <EmptyChart text="No session activity in this period." />
                ) : (
                  <ResponsiveContainer
                    width="100%"
                    height="100%"
                    minWidth={1}
                    initialDimension={{ width: 640, height: 224 }}
                  >
                    <AreaChart
                      data={daily}
                      margin={{ top: 10, right: 20, bottom: 4, left: 0 }}
                      accessibilityLayer
                    >
                      <ChartGradients
                        id={`${chartId}-sessions`}
                        colors={[
                          "var(--analytics-series-1)",
                          "var(--analytics-series-2)",
                        ]}
                      />
                      <CartesianGrid
                        vertical={false}
                        stroke="var(--analytics-grid)"
                        strokeWidth={0.6}
                        strokeOpacity={0.5}
                        strokeDasharray="4 6"
                      />
                      <XAxis
                        dataKey="day"
                        tickFormatter={dayLabel}
                        tick={chartTick}
                        tickLine={false}
                        axisLine={false}
                        minTickGap={35}
                        interval="preserveStartEnd"
                      />
                      <YAxis
                        allowDecimals={false}
                        width={40}
                        tick={chartTick}
                        tickLine={false}
                        axisLine={false}
                      />
                      <Tooltip
                        contentStyle={chartTooltipStyle}
                        itemStyle={chartTooltipTextStyle}
                        labelStyle={chartTooltipTextStyle}
                        labelFormatter={(label) =>
                          `${dayLabel(String(label))} UTC`
                        }
                      />
                      <Area
                        type="monotone"
                        dataKey="sessions"
                        name="Active sessions"
                        stroke="var(--analytics-series-1)"
                        fill={`url(#${chartId}-sessions-area-0)`}
                        fillOpacity={1}
                        strokeLinecap="round"
                        activeDot={{
                          r: 4.5,
                          fill: "var(--analytics-series-1)",
                          stroke: "var(--color-card)",
                          strokeWidth: 2,
                        }}
                        strokeWidth={2}
                        dot={
                          daily.length === 1 ? { r: 2, strokeWidth: 0 } : false
                        }
                        isAnimationActive={false}
                      />
                      <Area
                        type="monotone"
                        dataKey="turns"
                        name="Settled turns"
                        stroke="var(--analytics-series-2)"
                        fill={`url(#${chartId}-sessions-area-1)`}
                        fillOpacity={1}
                        strokeWidth={1.5}
                        strokeLinecap="round"
                        activeDot={{
                          r: 4.5,
                          fill: "var(--analytics-series-2)",
                          stroke: "var(--color-card)",
                          strokeWidth: 2,
                        }}
                        dot={
                          daily.length === 1 ? { r: 2, strokeWidth: 0 } : false
                        }
                        isAnimationActive={false}
                      />
                    </AreaChart>
                  </ResponsiveContainer>
                )}
              </div>
              <Legend />
            </Card>
            <Card className="analytics-panel analytics-chart min-w-0 p-4">
              <h3 className="font-display text-15 font-semibold leading-none tracking-tight">
                Feedback over time
              </h3>
              <p className="mb-4 mt-1 text-12 text-muted-foreground">
                Helpful and not helpful votes, by submission day.
              </p>
              <div
                className="analytics-plot h-56"
                aria-label="Daily reply feedback"
              >
                {!votes ? (
                  feedback.isPending ? (
                    <Skeleton className="h-full" />
                  ) : (
                    <p className="pt-16 text-center text-12 text-muted-foreground">
                      Feedback is unavailable.
                    </p>
                  )
                ) : !totalVotes ? (
                  <EmptyChart text="No feedback in this period." />
                ) : (
                  <ResponsiveContainer
                    width="100%"
                    height="100%"
                    minWidth={1}
                    initialDimension={{ width: 640, height: 224 }}
                  >
                    <BarChart
                      data={daily}
                      margin={{ top: 10, right: 20, bottom: 4, left: 0 }}
                      accessibilityLayer
                    >
                      <ChartGradients
                        id={`${chartId}-votes`}
                        colors={[
                          "var(--analytics-series-4)",
                          "var(--analytics-series-5)",
                        ]}
                      />
                      <CartesianGrid
                        vertical={false}
                        stroke="var(--analytics-grid)"
                        strokeWidth={0.6}
                        strokeOpacity={0.5}
                        strokeDasharray="4 6"
                      />
                      <XAxis
                        dataKey="day"
                        tickFormatter={dayLabel}
                        tick={chartTick}
                        tickLine={false}
                        axisLine={false}
                        minTickGap={35}
                        interval="preserveStartEnd"
                      />
                      <YAxis
                        allowDecimals={false}
                        width={40}
                        tick={chartTick}
                        tickLine={false}
                        axisLine={false}
                      />
                      <Tooltip
                        contentStyle={chartTooltipStyle}
                        itemStyle={chartTooltipTextStyle}
                        labelStyle={chartTooltipTextStyle}
                        cursor={{ fill: "var(--color-muted)", opacity: 0.4 }}
                        labelFormatter={(label) =>
                          `${dayLabel(String(label))} UTC`
                        }
                      />
                      <Bar
                        dataKey="up"
                        name="Helpful"
                        stackId="votes"
                        fill={`url(#${chartId}-votes-bar-0)`}
                        maxBarSize={24}
                        isAnimationActive={false}
                      />
                      <Bar
                        dataKey="down"
                        name="Not helpful"
                        stackId="votes"
                        fill={`url(#${chartId}-votes-bar-1)`}
                        maxBarSize={24}
                        radius={[3, 3, 0, 0]}
                        isAnimationActive={false}
                      />
                    </BarChart>
                  </ResponsiveContainer>
                )}
              </div>
              <Legend feedback />
            </Card>
          </div>
          {data && (
            <Card className="p-4">
              <div className="flex flex-wrap justify-between gap-4">
                <div>
                  <h3 className="font-display text-15 font-semibold leading-none tracking-tight">
                    Turn outcomes
                  </h3>
                  <p className="mt-1 text-11 text-text-tertiary">
                    {number(totalTurns)} settled turns · stopped turns are
                    counted separately from failures.
                  </p>
                </div>
                <div className="text-12 text-muted-foreground">
                  <span className="font-medium text-foreground">
                    {number(data.outcomes.with_tools)}
                  </span>{" "}
                  turns used tools <span className="mx-2">·</span>
                  <span className="font-medium text-foreground">
                    {number(data.outcomes.with_tool_errors)}
                  </span>{" "}
                  with recorded tool errors
                </div>
              </div>
              {totalTurns > 0 && (
                <div
                  className="mt-4 flex h-2 overflow-hidden rounded-full bg-overlay"
                  aria-hidden="true"
                >
                  <span
                    style={{
                      background: "var(--analytics-series-4)",
                      width: `${(data.outcomes.completed / totalTurns) * 100}%`,
                    }}
                  />
                  <span
                    style={{
                      background: "var(--analytics-series-5)",
                      width: `${(data.outcomes.failed / totalTurns) * 100}%`,
                    }}
                  />
                  <span
                    style={{
                      background: "var(--analytics-series-3)",
                      width: `${(data.outcomes.stopped / totalTurns) * 100}%`,
                    }}
                  />
                </div>
              )}
              <div className="mt-3 flex flex-wrap gap-5 text-12">
                <span>
                  Completed{" "}
                  <strong className="ml-1 text-success">
                    {number(data.outcomes.completed)}
                  </strong>
                </span>
                <span>
                  Failed{" "}
                  <strong className="ml-1 text-destructive">
                    {number(data.outcomes.failed)}
                  </strong>
                </span>
                <span>
                  Stopped{" "}
                  <strong className="ml-1 text-warning">
                    {number(data.outcomes.stopped)}
                  </strong>
                </span>
              </div>
            </Card>
          )}
          {(data || votes) && (
            <details className="text-11 text-muted-foreground">
              <summary className="w-fit cursor-pointer">
                View chart data
              </summary>
              <div className="mt-3 overflow-x-auto rounded-lg border border-border">
                <table className="w-full text-left text-12">
                  <caption className="sr-only">
                    Daily NyxBot activity and feedback in UTC
                  </caption>
                  <thead className="border-b border-border">
                    <tr>
                      {[
                        "Day (UTC)",
                        ...(data ? ["Active sessions", "Settled turns"] : []),
                        ...(votes ? ["Helpful", "Not helpful"] : []),
                      ].map((label) => (
                        <th key={label} className="px-3 py-2 font-medium">
                          {label}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {daily.map((row) => (
                      <tr
                        key={row.day}
                        className="border-b border-border/50 last:border-0"
                      >
                        <td className="px-3 py-2">{row.day}</td>
                        {data && (
                          <>
                            <td className="px-3 py-2">{row.sessions}</td>
                            <td className="px-3 py-2">{row.turns}</td>
                          </>
                        )}
                        {votes && (
                          <>
                            <td className="px-3 py-2">{row.up}</td>
                            <td className="px-3 py-2">{row.down}</td>
                          </>
                        )}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
          )}
          <AssistantFeedbackPanel
            key={`${from}:${to}`}
            range={feedbackParams}
          />
          <p className="text-11 leading-relaxed text-text-tertiary">
            Sessions are conversation threads with user input, including channel
            input. Turn outcomes include specialist and automated work; mirrored
            voice replies count once per turn. Feedback includes group replies.
            These reports use retained records; edits and deletions can change
            past totals. Tool indicators reflect recorded activities.
          </p>
        </>
      )}
    </div>
  );
}

function EmptyChart({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-12 text-muted-foreground">
      {text}
    </div>
  );
}

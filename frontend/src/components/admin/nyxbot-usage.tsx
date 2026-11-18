import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { Label } from "@/components/ui/label";
import { NyxbotRealWork } from "./nyxbot-real-work";
import { Input as ThresholdInput } from "@/components/ui/input";
import { NyxbotModelUsage } from "./nyxbot-model-usage";
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
import { ApiError } from "@/lib/api-client";
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

const number = (value: number) => value.toLocaleString(getFormattingLocale());
const dayLabel = (day: string) =>
  new Date(`${day}T00:00:00Z`).toLocaleDateString(getFormattingLocale(), {
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
  useUiTranslation();
  return (
    <Card className="min-w-0 px-4 py-4">
      <p className="type-group-label uppercase tracking-[1.5px] text-muted-foreground">
        {label}
      </p>
      <p className="mt-3 font-display type-metric tracking-tight tabular-nums">
        {value}
      </p>
      <p className="mt-2 text-12 text-muted-foreground">
        {detail}
      </p>
    </Card>
  );
}

function Legend({ feedback = false }: { feedback?: boolean }) {
  useUiTranslation();
  return (
    <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1.5 text-13 text-foreground">
      <span className="flex items-center gap-1.5">
        <span
          className="size-1.5 rounded-full"
          style={{
            background: feedback
              ? "var(--analytics-series-4)"
              : "var(--analytics-series-1)",
          }}
        />
        {feedback ? translateUi("Helpful") : translateUi("Active sessions")}
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
        {feedback ? translateUi("Not helpful") : translateUi("Settled turns")}
      </span>
    </div>
  );
}

export function NyxbotUsage() {
  useUiTranslation();
  const [threshold, setThreshold] = useState(3);
  const chartId = useId().replace(/:/g, "");
  const [dates, setDates] = useState(() => nyxbotDateRange(7));
  const window = nyxbotWindow(dates.from, dates.to);
  const from = window?.from ?? "";
  const to = window?.to ?? "";
  const analytics = useAssistantAnalytics(from, to, Boolean(window), threshold);
  // Feedback uses an inclusive upper bound; analytics uses an exclusive one.
  const feedbackParams = new URLSearchParams(
    window ? { from, to: new Date(Date.parse(to) - 1).toISOString() } : {},
  );
  const feedback = useAssistantFeedbackAdmin(feedbackParams, Boolean(window));
  const data = analytics.data;
  const feedbackDenied =
    feedback.error instanceof ApiError &&
    [403, 404].includes(feedback.error.status);
  const feedbackData = feedbackDenied ? undefined : feedback.data;
  const votes = feedbackData?.counts.reduce(
    (total, row) => ({ up: total.up + row.up, down: total.down + row.down }),
    { up: 0, down: 0 },
  );
  const totalVotes = votes ? votes.up + votes.down : 0;
  const daily = window ? nyxbotDailyData(from, to, data, feedbackData) : [];
  const outcomes = data?.outcomes;
  const totalTurns = outcomes
    ? outcomes.completed + outcomes.failed + outcomes.stopped
    : 0;
  return (
    <div className="analytics-visuals space-y-4">
      <div>
        <h2 className="font-display type-card-title tracking-tight">{translateUi("NyxBot usage & feedback")}</h2>
        <p className="mt-1 text-14 text-muted-foreground">{translateUi("Understand how NyxBot and specialists are used, where work fails, and what users say.")}</p>
      </div>
      <FilterCard
        pickers={
          <div
            className="flex flex-wrap items-end gap-2"
            aria-label={translateUi("NyxBot date range")}
          >
            <Button
              variant="outline"
              onClick={() => setDates(nyxbotDateRange(7))}
            >{translateUi("Last 7 days")}</Button>
            <Button
              variant="outline"
              onClick={() => setDates(nyxbotDateRange(30))}
            >{translateUi("Last 30 days")}</Button>
            <Label className="space-y-1 text-muted-foreground">{translateUi("From")}<Input
                type="date"
                value={dates.from}
                onChange={(event) =>
                  setDates({ ...dates, from: event.target.value })
                }
              />
            </Label>
            <Label className="space-y-1 text-muted-foreground">{translateUi("To")}<Input
                type="date"
                value={dates.to}
                onChange={(event) =>
                  setDates({ ...dates, to: event.target.value })
                }
              />
            </Label>
            <span className="pb-2 text-13 text-text-tertiary">{translateUi("UTC")}</span>
          </div>
        }
        aside={
          <span className="text-12 text-muted-foreground">{translateUi("Daily · UTC")}</span>
        }
      />
      {!window ? (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 p-3 text-14 text-destructive"
        >{translateUi("Choose a start and end date covering at most 31 days.")}</p>
      ) : (
        <>
          <div
            className="flex flex-wrap items-center gap-2 text-13 text-text-tertiary"
            role="status"
          >
            <span
              className={`size-1.5 rounded-full ${data && !analytics.isError ? "bg-success" : "bg-text-tertiary"}`}
              aria-hidden="true"
            />
            {analytics.isFetching
              ? translateUi("Updating activity…")
              : data
                ? translateUi("Updated {{value1}} · live count refreshes every 30 seconds", { value1: new Date(data.generated_at).toLocaleTimeString(getFormattingLocale()) })
                : analytics.isPending
                  ? translateUi("Loading activity…")
                  : translateUi("Activity unavailable")}
          </div>
          {analytics.isError && (
            <div role="alert">
              <ErrorBanner
                message={translateUi("Unable to load NyxBot activity.")}
                onRetry={() => void analytics.refetch()}
              />
            </div>
          )}
          {feedback.isError && (
            <div role="alert">
              <ErrorBanner
                message={
                  feedbackData
                    ? translateUi("Showing previously loaded feedback totals. Refresh failed.")
                    : translateUi("Unable to load feedback totals.")
                }
                onRetry={() => void feedback.refetch()}
              />
            </div>
          )}
          <div className="analytics-summary grid grid-cols-2 lg:grid-cols-4">
            <Stat
              label={translateUi("Active sessions")}
              value={data ? number(data.active_sessions) : "—"}
              detail={
                data
                  ? `Conversations with user input · ${number(data.active_accounts)} accounts`
                  : "Conversations with user input in this period"
              }
            />
            <Stat
              label={translateUi("Running now")}
              value={data ? number(data.running_now) : "—"}
              detail="Live turns with a recent worker heartbeat"
            />
            <Stat
              label={translateUi("Feedback received")}
              value={votes ? number(totalVotes) : "—"}
              detail={
                votes
                  ? `${number(votes.up)} helpful · ${number(votes.down)} not helpful`
                  : "Retained votes submitted in this period"
              }
            />
            <Stat
              label={translateUi("Helpful feedback")}
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
          <p className="type-secondary text-text-tertiary">{translateUi("Helpful rate reflects people who chose to leave feedback, not task success. Review a rating and preview its conversation to understand what worked or needs attention.")}</p>
          <AssistantFeedbackPanel
            key={`${from}:${to}`}
            range={feedbackParams}
          />
          <div className="grid min-w-0 gap-3 lg:grid-cols-2">
            <Card className="analytics-panel analytics-chart min-w-0 p-4">
              <h3 className="font-display type-card-title tracking-tight">{translateUi("Session activity")}</h3>
              <p className="mb-4 mt-1 text-14 text-muted-foreground">{translateUi("Daily conversations with user input and unique settled turns.")}</p>
              <div
                className="analytics-plot h-56"
                aria-label={translateUi("Daily session activity")}
              >
                {!data ? (
                  analytics.isPending ? (
                    <Skeleton className="h-full" />
                  ) : (
                    <p className="pt-16 text-center text-14 text-muted-foreground">{translateUi("Activity is unavailable.")}</p>
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
              <h3 className="font-display type-card-title tracking-tight">{translateUi("Feedback over time")}</h3>
              <p className="mb-4 mt-1 text-14 text-muted-foreground">{translateUi("Helpful and not helpful votes, by submission day.")}</p>
              <div
                className="analytics-plot h-56"
                aria-label={translateUi("Daily reply feedback")}
              >
                {!votes ? (
                  feedback.isPending ? (
                    <Skeleton className="h-full" />
                  ) : (
                    <p className="pt-16 text-center text-14 text-muted-foreground">{translateUi("Feedback is unavailable.")}</p>
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
                  <h3 className="font-display type-card-title tracking-tight">{translateUi("Turn outcomes")}</h3>
                  <p className="mt-1 text-13 text-text-tertiary">
                    {number(totalTurns)}{translateUi(" settled turns · stopped turns are counted separately from failures.")}</p>
                </div>
                <div className="text-14 text-muted-foreground">
                  <span className="font-medium text-foreground">
                    {number(data.outcomes.with_tools)}
                  </span>{" "}{translateUi("turns used tools ")}<span className="mx-2">·</span>
                  <span className="font-medium text-foreground">
                    {number(data.outcomes.with_tool_errors)}
                  </span>{" "}{translateUi("with recorded tool errors")}</div>
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
              <div className="mt-3 flex flex-wrap gap-5 text-14">
                <span>{translateUi("Completed")}{" "}
                  <strong className="ml-1 text-success">
                    {number(data.outcomes.completed)}
                  </strong>
                </span>
                <span>{translateUi("Failed")}{" "}
                  <strong className="ml-1 text-destructive">
                    {number(data.outcomes.failed)}
                  </strong>
                </span>
                <span>{translateUi("Stopped")}{" "}
                  <strong className="ml-1 text-warning">
                    {number(data.outcomes.stopped)}
                  </strong>
                </span>
              </div>
            </Card>
          )}
          {(data || votes) && (
            <details className="text-13 text-muted-foreground">
              <summary className="w-fit cursor-pointer">{translateUi("View chart data")}</summary>
              <div className="mt-3 overflow-x-auto rounded-lg border border-border">
                <table className="w-full text-left text-14">
                  <caption className="sr-only">{translateUi("Daily NyxBot activity and feedback in UTC")}</caption>
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
          <div className="flex items-center gap-3 text-14">
            <Label className="flex items-center gap-2">{translateUi("Weekly work threshold")}<ThresholdInput
                aria-label={translateUi("Weekly work threshold")}
                type="number"
                min={1}
                max={1000}
                className="w-20"
                value={threshold}
                onChange={(event) => {
                  const value = Number(event.target.value);
                  if (Number.isInteger(value) && value >= 1 && value <= 1000)
                    setThreshold(value);
                }}
              />
            </Label>
          </div>
          {data?.real_work && <NyxbotRealWork data={data.real_work} />}
          {data?.real_work_unavailable && (
            <Card className="space-y-3 p-4">
              <h3 className="font-display type-card-title">{translateUi("Weekly real work")}</h3>
              <ErrorBanner
                message={translateUi("Weekly real work is unavailable.")}
                onRetry={() => void analytics.refetch()}
              />
            </Card>
          )}
          <NyxbotModelUsage from={from} to={to} />
          <p className="type-secondary text-text-tertiary">{translateUi("Sessions are conversation threads with user input, including channel input. Turn outcomes include specialist and automated work; mirrored voice replies count once per turn. Feedback includes group replies. These reports use retained records; edits and deletions can change past totals. Tool indicators reflect recorded activities.")}</p>
        </>
      )}
    </div>
  );
}

function EmptyChart({ text }: { text: string }) {
  useUiTranslation();
  return (
    <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-14 text-muted-foreground">
      {text}
    </div>
  );
}

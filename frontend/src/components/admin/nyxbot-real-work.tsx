import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { WeeklyRealWork } from "@/schemas/assistant-real-work";
import {
  internalOwnersFormSchema,
  splitOwnerIds,
} from "@/schemas/assistant-real-work";
import { useAssistantInternalOwners } from "@/hooks/use-assistant-internal-owners";
import { useAuthStore } from "@/stores/auth-store";
import { hasAdminRead } from "@/types/api";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  chartTick,
  chartTooltipStyle,
  chartTooltipTextStyle,
} from "@/components/billing-analytics/chart-style";

const rate = (n: number, d: number) =>
  d ? `${Math.round((n / d) * 100)}%` : "—";
const elapsed = (seconds: number | null) =>
  seconds === null
    ? "—"
    : seconds < 3600
      ? `${Math.round(seconds / 60)} min`
      : seconds < 86400
        ? `${(seconds / 3600).toFixed(1)} hr`
        : `${(seconds / 86400).toFixed(1)} days`;

export function NyxbotRealWork({ data }: { data: WeeklyRealWork }) {
  useUiTranslation();
  return (
    <Card className="space-y-4 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="font-display type-card-title">{translateUi("Weekly real work")}</h3>
          <p className="mt-1 text-14 text-muted-foreground">{translateUi("Merchants with at least ")}{data.threshold}{translateUi(" completed tasks using their own connected business systems.")}</p>
          <p className="mt-1 text-13 text-text-tertiary">{translateUi("Monday–Sunday · Asia/Singapore · partial weeks use only the selected dates.")}</p>
        </div>
        <InternalOwners />
      </div>
      <div
        className="analytics-plot h-48"
        aria-label={translateUi("Weekly merchants and internal owners")}
      >
        <ResponsiveContainer
          width="100%"
          height="100%"
          minWidth={1}
          initialDimension={{ width: 640, height: 192 }}
        >
          <BarChart data={data.weeks} accessibilityLayer>
            <CartesianGrid
              vertical={false}
              stroke="var(--analytics-grid)"
              strokeDasharray="4 6"
            />
            <XAxis
              dataKey="week"
              tick={chartTick}
              tickLine={false}
              axisLine={false}
            />
            <YAxis
              allowDecimals={false}
              tick={chartTick}
              tickLine={false}
              axisLine={false}
              width={35}
            />
            <Tooltip
              contentStyle={chartTooltipStyle}
              itemStyle={chartTooltipTextStyle}
              labelStyle={chartTooltipTextStyle}
            />
            <Bar
              dataKey="merchants"
              name="Merchants"
              fill="var(--analytics-series-1)"
              maxBarSize={28}
              isAnimationActive={false}
            />
            <Bar
              dataKey="internal_owners"
              name="Internal owners"
              fill="var(--analytics-series-2)"
              maxBarSize={28}
              isAnimationActive={false}
            />
          </BarChart>
        </ResponsiveContainer>
      </div>
      <Table className="text-14" aria-label={translateUi("Weekly real work totals")}>
        <TableHeader>
          <TableRow>
            {[
              "Week starting",
              `Merchants ≥ ${data.threshold}`,
              `Internal ≥ ${data.threshold}`,
              "Work completed",
              "From triggers",
              "Success",
              "Helpful feedback",
            ].map((h) => (
              <TableHead key={h}>{h}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {data.weeks.map((w) => (
            <TableRow key={w.week}>
              <TableCell>
                {w.week}
                {w.partial && (
                  <span className="ml-2 text-12 text-text-tertiary">{translateUi("Partial")}</span>
                )}
              </TableCell>
              <TableCell className="font-normal tabular-nums">
                {w.merchants}
              </TableCell>
              <TableCell>{w.internal_owners}</TableCell>
              <TableCell>{w.completed}</TableCell>
              <TableCell>{rate(w.triggered, w.completed)}</TableCell>
              <TableCell>{rate(w.completed, w.attempted)}</TableCell>
              <TableCell>
                {rate(w.helpful, w.votes)}{" "}
                <span className="text-text-tertiary">({w.votes}{translateUi(" votes)")}</span>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {data.weeks.map((w) => (
        <details key={w.week} className="text-14">
          <summary className="cursor-pointer">
            {w.week}{translateUi(" · Owners and activation")}</summary>
          <div className="mt-3 space-y-4">
            <p className="text-13 text-muted-foreground">{translateUi("Internal owners: ")}{w.internal_completed}{translateUi(" completed ·")}{" "}
              {rate(w.internal_triggered, w.internal_completed)}{translateUi(" from triggers ·")}{" "}
              {rate(w.internal_completed, w.internal_attempted)}{translateUi(" success ·")}{" "}
              {rate(w.internal_helpful, w.internal_votes)}{translateUi(" helpful (")}{w.internal_votes}{translateUi(" votes).")}</p>
            {w.owners.length ? (
              <Table
                className="text-14"
                aria-label={translateUi("Work per owner {{value1}}", { value1: w.week })}
              >
                <TableHeader>
                  <TableRow>
                    {[
                      "Owner ID",
                      "Group",
                      "Completed",
                      "From triggers",
                      "Success",
                      "Helpful",
                    ].map((h) => (
                      <TableHead key={h}>{h}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {w.owners.map((o) => (
                    <TableRow key={o.owner_id}>
                      <TableCell className="font-mono text-14">
                        {o.owner_id}
                      </TableCell>
                      <TableCell>
                        {o.internal ? translateUi("Internal") : translateUi("Merchant")}
                      </TableCell>
                      <TableCell>{o.completed}</TableCell>
                      <TableCell>{rate(o.triggered, o.completed)}</TableCell>
                      <TableCell>{rate(o.completed, o.attempted)}</TableCell>
                      <TableCell>{rate(o.helpful, o.votes)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <p className="rounded-lg border border-dashed border-border p-3 text-muted-foreground">{translateUi("No real work recorded in this week.")}</p>
            )}
            {w.owners_truncated && (
              <p className="text-13 text-text-tertiary">{translateUi("Showing the top 100 owners. Weekly totals include every owner.")}</p>
            )}
            <h4 className="font-medium">{translateUi("New-owner activation")}</h4>
            {w.funnel.length ? (
              <Table
                className="text-14"
                aria-label={translateUi("Activation funnel {{value1}}", { value1: w.week })}
              >
                <TableHeader>
                  <TableRow>
                    {[
                      "Group",
                      "Created",
                      "Has NyxBot",
                      "Business connected",
                      "Channel work assigned",
                      "First real work",
                      "Median to first work",
                    ].map((h) => (
                      <TableHead key={h}>{h}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {w.funnel.map((f) => (
                    <TableRow key={String(f.internal)}>
                      <TableCell>
                        {f.internal ? translateUi("Internal") : translateUi("Merchants")}
                      </TableCell>
                      <TableCell>{f.owners}</TableCell>
                      <TableCell>{f.has_nyxbot}</TableCell>
                      <TableCell>{f.connected_business}</TableCell>
                      <TableCell>{f.assigned_channel_work}</TableCell>
                      <TableCell>{f.first_real_work}</TableCell>
                      <TableCell>
                        {elapsed(f.median_seconds_to_first_work)}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <p className="text-muted-foreground">{translateUi("No owners created in this selected portion of the week.")}</p>
            )}
          </div>
        </details>
      ))}
      <p className="text-13 text-text-tertiary">{translateUi("Success is completed tasks divided by settled tasks with a successful business call, including tasks that later failed or stopped. Helpful feedback uses votes on completed real work. Model calls, platform credentials and chat alone do not count. Each triggered run and mirrored voice reply counts once.")}</p>
      <p className="text-13 text-text-tertiary">{translateUi("Activation includes each prior stage, observed through the selected end date; organization assistants count as having NyxBot. The median is estimated for owners reaching every stage. Reports use retained records collected since real-work tracking began. Internal classification applies to past weeks too.")}</p>
    </Card>
  );
}

function InternalOwners() {
  useUiTranslation();
  const user = useAuthStore((s) => s.user);
  const [open, setOpen] = useState(false);
  const { query, save } = useAssistantInternalOwners(open);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Button variant="outline" onClick={() => setOpen(true)}>{translateUi("Internal owners")}</Button>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Internal and test owners")}</DialogTitle>
          <DialogDescription>{translateUi("These owners are reported separately from merchants in every week.")}</DialogDescription>
        </DialogHeader>
        {query.isPending ? (
          <p className="text-14">{translateUi("Loading owners…")}</p>
        ) : query.isError ? (
          <ErrorBanner
            message={translateUi("Could not load internal owners.")}
            onRetry={() => void query.refetch()}
          />
        ) : hasAdminRead(user) ? (
          <InternalOwnersForm
            initial={query.data.owner_ids}
            save={save}
            onSaved={() => setOpen(false)}
          />
        ) : (
          <p className="max-h-72 overflow-auto whitespace-pre-wrap font-mono text-13">
            {query.data.owner_ids.join("\n") ||
              translateUi("No internal owners configured.")}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
function InternalOwnersForm({
  initial,
  save,
  onSaved,
}: {
  initial: string[];
  save: ReturnType<typeof useAssistantInternalOwners>["save"];
  onSaved: () => void;
}) {
  useUiTranslation();
  const form = useAppForm({
    resolver: zodResolver(internalOwnersFormSchema),
    defaultValues: { owners: initial.join("\n") },
  });
  return (
    <Form {...form}>
      <form
        className="space-y-4"
        onSubmit={form.handleSubmit(async (value) => {
          try {
            await save.mutateAsync(splitOwnerIds(value.owners));
            onSaved();
          } catch {
            /* Mutation error remains visible below. */
          }
        })}
      >
        <FormField
          control={form.control}
          name="owners"
          render={({ field }) => (
            <FormItem>
              <FormLabel>{translateUi("Person or organization IDs")}</FormLabel>
              <FormControl>
                <Textarea
                  {...field}
                  rows={8}
                  placeholder={translateUi("One owner UUID per line")}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {save.isError && (
          <ErrorBanner message={translateUi("Could not save. Check that every ID is an existing person or organization.")} />
        )}
        <Button
          type="submit"
          disabled={save.isPending || !form.formState.isDirty}
        >
          {save.isPending ? translateUi("Saving…") : translateUi("Save internal owners")}
        </Button>
      </form>
    </Form>
  );
}

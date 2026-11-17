import { useState } from "react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { formatCreditAmount } from "@/lib/credits";
import { useAuthStore } from "@/stores/auth-store";
import {
  useManagedLineEffects,
  useManagedLinesReport,
} from "@/hooks/use-admin-managed-lines";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { EvidenceSection } from "@/components/admin/managed-lines/accounts-section";
import { when } from "@/components/admin/managed-lines/format";

const DAY = 86_400_000;
function utcDay(offset: number): string {
  const now = new Date();
  return new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()) + offset * DAY,
  )
    .toISOString()
    .slice(0, 10);
}
const midnight = (day: string) => `${day}T00:00:00Z`;
const money = (value: string | null) => (value == null ? "—" : formatCreditAmount(value));

/** Estimated vs actual margin; actual appears only for fully reconciled effects. */
export function AdminManagedLinesReportPage() {
  useUiTranslation();
  const user = useAuthStore((s) => s.user);
  const [from, setFrom] = useState(() => utcDay(-7));
  const [until, setUntil] = useState(() => utcDay(1));
  const [lineId, setLineId] = useState("");
  const [range, setRange] = useState({ from: midnight(from), until: midnight(until), lineId: "" });
  const [effects, setEffects] = useState<string>();
  const [resolve, setResolve] = useState<string>();
  const report = useManagedLinesReport({ ...range, lineId: range.lineId || undefined });
  const days = (Date.parse(midnight(until)) - Date.parse(midnight(from))) / DAY;
  const valid = days > 0 && days <= 31;
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Managed lines report")}
        description={translateUi("Billed credits, supplier cost and margin per line and UTC day. Margin is estimated whenever any cost component is a tariff estimate or unknown, and actual only when every component is a reconciled provider record.")}
      />
      <Card>
        <CardContent className="flex flex-wrap items-end gap-3 py-4">
          <Label className="space-y-1">
            <span>{translateUi("From (UTC)")}</span>
            <Input type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
          </Label>
          <Label className="space-y-1">
            <span>{translateUi("Until (UTC, exclusive)")}</span>
            <Input type="date" value={until} onChange={(e) => setUntil(e.target.value)} />
          </Label>
          <Label className="space-y-1">
            <span>{translateUi("Line ID (optional)")}</span>
            <Input value={lineId} onChange={(e) => setLineId(e.target.value.trim())} />
          </Label>
          <Button
            variant="outline"
            disabled={!valid}
            onClick={() => {
              setEffects(undefined);
              setRange({ from: midnight(from), until: midnight(until), lineId });
            }}
          >
            {translateUi("Show report")}
          </Button>
          {!valid && <p className="text-destructive">{translateUi("Choose a window of 1 to 31 days.")}</p>}
        </CardContent>
      </Card>
      {report.error && (
        <ErrorBanner
          message={translateUi("Could not load the managed lines report.")}
          onRetry={() => void report.refetch()}
        />
      )}
      {report.data?.pages.flatMap((p) => p.lines).map((line) => (
        <Card key={line.line_id}>
          <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-2 space-y-0">
            <CardTitle className="font-mono">{line.line_id}</CardTitle>
            <div className="flex flex-wrap gap-2">
              {line.inventory_policy_missing && (
                <Badge variant="warning">{translateUi("Inventory cost policy missing")}</Badge>
              )}
              <Button variant="outline" onClick={() => setEffects(line.line_id)}>
                {translateUi("Effects")}
              </Button>
            </div>
          </CardHeader>
          <CardContent className="space-y-4 overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{translateUi("Day")}</TableHead>
                  <TableHead>{translateUi("Effects")}</TableHead>
                  <TableHead>{translateUi("Billed")}</TableHead>
                  <TableHead>{translateUi("Provider cost")}</TableHead>
                  <TableHead>{translateUi("Estimated cost")}</TableHead>
                  <TableHead>{translateUi("Inventory cost")}</TableHead>
                  <TableHead>{translateUi("Estimated margin")}</TableHead>
                  <TableHead>{translateUi("Actual margin")}</TableHead>
                  <TableHead>{translateUi("Unknown components")}</TableHead>
                  <TableHead>{translateUi("Absorbed seconds")}</TableHead>
                  <TableHead>{translateUi("Absorbed SMS parts")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {line.days.map((d) => (
                  <TableRow key={d.day}>
                    <TableCell className="font-mono">{d.day}</TableCell>
                    <TableCell>{d.effect_count}</TableCell>
                    <TableCell>{money(d.billed_credits)}</TableCell>
                    <TableCell>{money(d.provider_actual_cost_credits)}</TableCell>
                    <TableCell>{money(d.estimated_cost_credits)}</TableCell>
                    <TableCell>{money(d.inventory_cost_credits)}</TableCell>
                    <TableCell>{money(d.estimated_margin_credits)}</TableCell>
                    <TableCell>{money(d.actual_margin_credits)}</TableCell>
                    <TableCell>{d.unknown_components}</TableCell>
                    <TableCell>{d.absorbed_seconds}</TableCell>
                    <TableCell>{d.absorbed_sms_parts}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            {line.budgets.length > 0 && (
              <div className="space-y-1">
                {line.budgets.map((b) => (
                  <p key={b.kind} className="text-muted-foreground">
                    {translateUi("{{kind}} budget: {{recognized}} recognized and {{holds}} held of {{limit}}", {
                      kind: b.kind,
                      recognized: formatCreditAmount(b.recognized),
                      holds: formatCreditAmount(b.unresolved_holds),
                      limit: formatCreditAmount(b.limit),
                    })}
                  </p>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      ))}
      {report.data && report.data.pages.every((p) => p.lines.length === 0) && (
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            {translateUi("No managed lines in this window.")}
          </CardContent>
        </Card>
      )}
      {report.hasNextPage && (
        <Button
          variant="outline"
          disabled={report.isFetchingNextPage}
          onClick={() => void report.fetchNextPage()}
        >
          {translateUi("Load more lines")}
        </Button>
      )}
      {effects && (
        <Effects
          range={{ from: range.from, until: range.until, lineId: effects }}
          canResolve={!!user?.is_admin}
          onResolve={setResolve}
        />
      )}
      {resolve && user?.is_admin && <EvidenceSection key={resolve} sessionId={resolve} />}
    </div>
  );
}

function Effects({
  range,
  canResolve,
  onResolve,
}: {
  readonly range: { from: string; until: string; lineId: string };
  readonly canResolve: boolean;
  readonly onResolve: (sessionId: string) => void;
}) {
  useUiTranslation();
  const page = useManagedLineEffects(range);
  const rows = page.data?.pages.flatMap((p) => p.effects) ?? [];
  return (
    <Card>
      <CardHeader>
        <CardTitle>{translateUi("Effects for {{line}}", { line: range.lineId })}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 overflow-x-auto">
        {page.error && <ErrorBanner message={translateUi("Could not load effects.")} onRetry={() => void page.refetch()} />}
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{translateUi("Effect")}</TableHead>
              <TableHead>{translateUi("Kind")}</TableHead>
              <TableHead>{translateUi("Created")}</TableHead>
              <TableHead>{translateUi("Coverage")}</TableHead>
              <TableHead>{translateUi("Evidence")}</TableHead>
              <TableHead>{translateUi("Components")}</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((e) => (
              <TableRow key={e.id}>
                <TableCell className="font-mono">{e.effect_id}</TableCell>
                <TableCell>{e.kind ?? "—"}</TableCell>
                <TableCell>{when(e.created_at)}</TableCell>
                <TableCell>
                  {e.component_coverage ? translateUi("Complete") : translateUi("Incomplete")}
                </TableCell>
                <TableCell>
                  {e.evidence_quality === "actual" ? translateUi("Actual") : translateUi("Estimated")}
                </TableCell>
                <TableCell className="font-mono">
                  {e.original_costs
                    .map((c) => `${c.kind}:${c.source}:${c.amount ?? "?"} ${c.currency}`)
                    .join(", ")}
                </TableCell>
                <TableCell>
                  {canResolve && e.kind === "call" && (
                    <Button variant="outline" onClick={() => onResolve(e.effect_id)}>
                      {translateUi("Resolve evidence")}
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        {page.hasNextPage && (
          <Button
            variant="outline"
            disabled={page.isFetchingNextPage}
            onClick={() => void page.fetchNextPage()}
          >
            {translateUi("Load more effects")}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}

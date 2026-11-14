import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "@/components/ui/table";
import { formatDateTime } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import { useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useAppForm } from "@/components/ui/form";
import { useUiTranslation, translateUi as t } from "@/i18n/use-ui-translation";
import {
  useCrashReport,
  useCrashReports,
  useCrashSummary,
} from "@/hooks/use-admin-crash-reports";
import {
  crashKindSchema,
  type CrashFilters,
  type CrashReportDetail,
} from "@/schemas/admin-crash-reports";
const filterSchema = z.object({
  platform: z.string(),
  app_version: z.string().max(32),
  app_build: z.string().max(64),
  kind: z.string(),
  fingerprint: z.string().max(64),
});
const outcomeLabels: Record<string, string> = {
  accepted: "Accepted",
  duplicate: "Duplicate",
  consent_denied: "Consent denied",
  unavailable: "Unavailable",
  rate_limited: "Rate limited",
  invalid: "Invalid",
};
const kindLabels: Record<string, string> = {
  native_crash: "iOS native crash",
  js_fatal: "JavaScript fatal error",
  js_error: "JavaScript error",
  unhandled_rejection: "Unhandled rejection",
  render_error: "Render error",
  recoverable_error: "Recoverable error",
  resource_error: "Resource error",
};
const exportLabels: Record<string, string> = {
  disabled: "Disabled",
  pending: "Pending",
  dispatching: "Dispatching",
  exported: "Exported",
  unknown: "Unknown",
  failed: "Failed",
  discarded: "Discarded",
};
const platformLabels: Record<string, string> = {
  web: "Web",
  ios: "iOS",
  android: "Android",
};
const windowLabels: Record<string, string> = {
  "24h": "Last 24 hours",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
};
function label(value: string, labels: Record<string, string>) {
  return t(labels[value] ?? value);
}
function LoadState({ error, loading }: { error: unknown; loading: boolean }) {
  return error ? (
    <p role="alert">{t("Could not load crash reports.")}</p>
  ) : loading ? (
    <p>{t("Loading…")}</p>
  ) : null;
}
export function AdminCrashReportsPage() {
  useUiTranslation();
  const [reportWindow, setReportWindow] = useState("24h");
  const [filters, setFilters] = useState<CrashFilters>({});
  const form = useAppForm({
    resolver: zodResolver(filterSchema),
    defaultValues: {
      platform: "",
      app_version: "",
      app_build: "",
      kind: "",
      fingerprint: "",
    },
  });
  const summary = useCrashSummary(reportWindow, filters.platform ?? "");
  const reports = useCrashReports(filters);
  return (
    <div className="space-y-6">
      <PageHeader
        title={t("Crash reports")}
        description={t(
          "Reported events by version. Counts reflect consented reports, not all crashes or affected people.",
        )}
      />
      <form
        className="flex flex-wrap items-end gap-3"
        onSubmit={form.handleSubmit((values) => setFilters(values))}
      >
        <Label>
          {t("Platform")}
          <NativeSelect
            aria-label={t("Platform")}
            {...form.register("platform")}
          >
            <option value="">{t("All platforms")}</option>
            {Object.entries(platformLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {t(label)}
              </option>
            ))}
          </NativeSelect>
        </Label>
        <Label>
          {t("Version")}
          <Input
            aria-label={t("Version")}
            maxLength={32}
            {...form.register("app_version")}
          />
        </Label>
        <Label>
          {t("Build")}
          <Input
            aria-label={t("Build")}
            maxLength={64}
            {...form.register("app_build")}
          />
        </Label>
        <Label>
          {t("Kind")}
          <NativeSelect aria-label={t("Kind")} {...form.register("kind")}>
            <option value="">{t("All kinds")}</option>
            {crashKindSchema.options.map((k) => (
              <option key={k} value={k}>
                {label(k, kindLabels)}
              </option>
            ))}
          </NativeSelect>
        </Label>
        <Label>
          {t("Fingerprint")}
          <Input
            aria-label={t("Fingerprint")}
            maxLength={64}
            {...form.register("fingerprint")}
          />
        </Label>
        <Button type="submit" variant="outline">
          {t("Apply filters")}
        </Button>
        <Label>
          {t("Window")}
          <NativeSelect
            aria-label={t("Window")}
            value={reportWindow}
            onChange={(e) => setReportWindow(e.target.value)}
          >
            {Object.entries(windowLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {t(label)}
              </option>
            ))}
          </NativeSelect>
        </Label>
      </form>
      <LoadState error={summary.error} loading={summary.isLoading} />
      {summary.data && (
        <>
          <Card>
            <CardHeader>
              <CardTitle>{t("Ingestion health")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4 type-body">
              <dl className="grid grid-cols-2 gap-3 md:grid-cols-6">
                {Object.entries(outcomeLabels).map(([key, value]) => (
                  <div key={key}>
                    <dt className="text-muted-foreground">{t(value)}</dt>
                    <dd className="font-mono type-metric">
                      {summary.data.health.outcomes[key] ?? 0}
                    </dd>
                  </div>
                ))}
              </dl>
              <p>
                {t("Retention")}:{" "}
                {t("{{days}} days", {
                  days: summary.data.health.retention_days,
                })}{" "}
                · {t("Reporting rollout")}:{" "}
                {summary.data.health.flag_enabled
                  ? t("Enabled")
                  : t("Disabled")}
              </p>
              <p>
                {t("Export destinations")}:{" "}
                {summary.data.health.destination_ids.join(", ") || t("None")}
              </p>
              <dl className="flex flex-wrap gap-4">
                {Object.entries(summary.data.health.export_statuses).map(
                  ([key, count]) => (
                    <div key={key}>
                      <dt>{label(key, exportLabels)}</dt>
                      <dd className="font-mono">{count}</dd>
                    </div>
                  ),
                )}
              </dl>
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>{t("Reported events by version")}</CardTitle>
            </CardHeader>
            <CardContent className="overflow-x-auto">
              <Table className="w-full text-left type-table-cell">
                <TableHeader>
                  <TableRow>
                    {[
                      "Platform",
                      "Environment",
                      "Version",
                      "Build",
                      "Artifact",
                      "Reported events",
                      "Reported events by kind",
                      "Last seen",
                    ].map((h) => (
                      <TableHead className="p-2 type-table-header" key={h}>
                        {t(h)}
                      </TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {summary.data.versions.map((v, i) => (
                    <TableRow key={i} className="border-t border-border">
                      <TableCell className="p-2">
                        {label(v.platform, platformLabels)}
                      </TableCell>
                      <TableCell>{v.environment}</TableCell>
                      <TableCell>
                        <Button
                          variant="link"
                          onClick={() => {
                            form.reset({
                              platform: v.platform,
                              app_version: v.app_version,
                              app_build: v.app_build,
                              kind: "",
                              fingerprint: "",
                            });
                            setFilters({
                              platform: v.platform,
                              app_version: v.app_version,
                              app_build: v.app_build,
                            });
                          }}
                        >
                          {v.app_version}
                        </Button>
                      </TableCell>
                      <TableCell className="font-mono">{v.app_build}</TableCell>
                      <TableCell className="font-mono">{v.artifact_id ?? "—"}</TableCell>
                      <TableCell>{v.total}</TableCell>
                      <TableCell>
                        {Object.entries(v.counts)
                          .filter(([, n]) => n > 0)
                          .map(([k, n]) => `${label(k, kindLabels)}: ${n}`)
                          .join(", ")}
                      </TableCell>
                      <TableCell className="font-mono">{formatDateTime(v.last_seen)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </>
      )}
      <Card>
        <CardHeader>
          <CardTitle>{t("Individual reports")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 overflow-x-auto">
          <LoadState error={reports.error} loading={reports.isLoading} />
          {reports.data?.reports.length === 0 && (
            <p>{t("No reports in this selection.")}</p>
          )}
          <Table className="w-full text-left type-table-cell">
            <TableHeader>
              <TableRow>
                {[
                  "Captured",
                  "Platform",
                  "Version",
                  "Kind",
                  "Export status",
                ].map((h) => (
                  <TableHead className="p-2 type-table-header" key={h}>
                    {t(h)}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {reports.data?.reports.map((r) => (
                <TableRow key={r.id} className="border-t border-border">
                  <TableCell className="p-2 font-mono">
                    <Link
                      to="/admin/crash-reports/$id"
                      params={{ id: r.id }}
                      className="underline"
                    >
                      {formatDateTime(r.captured_at)}
                    </Link>
                  </TableCell>
                  <TableCell>{label(r.platform, platformLabels)}</TableCell>
                  <TableCell>
                    {r.app_version} ({r.app_build})
                  </TableCell>
                  <TableCell>{label(r.kind, kindLabels)}</TableCell>
                  <TableCell>{label(r.export.status, exportLabels)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {reports.data?.cursor && (
            <Button
              variant="outline"
              onClick={() =>
                setFilters({ ...filters, cursor: reports.data!.cursor! })
              }
            >
              {t("Next page")}
            </Button>
          )}
          {filters.cursor && (
            <Button
              variant="outline"
              onClick={() => setFilters({ ...filters, cursor: undefined })}
            >
              {t("First page")}
            </Button>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
export function CrashReportView({ report: r }: { report: CrashReportDetail }) {
  useUiTranslation();
  return (
    <Card>
      <CardContent className="space-y-4 pt-6 type-body">
        <dl className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {Object.entries({
            [t("Report ID")]: r.id,
            [t("Platform")]: label(r.platform, platformLabels),
            [t("Version")]: r.app_version,
            [t("Build")]: r.app_build,
            [t("Artifact")]: r.artifact_id ?? "—",
            [t("Environment")]: r.environment,
            [t("Kind")]: label(r.kind, kindLabels),
            [t("Exception class")]: r.exception_class,
            [t("Signal")]: r.native_signal ?? "—",
            [t("OS version")]: r.os_version,
            [t("Runtime")]: r.runtime
              ? `${r.runtime.name} ${r.runtime.major}`
              : "—",
            [t("Fingerprint")]: r.fingerprint,
            [t("Captured")]: formatDateTime(r.captured_at),
            [t("Received")]: formatDateTime(r.received_at),
            [t("Expires")]: formatDateTime(r.expires_at),
            [t("Export status")]: label(r.export.status, exportLabels),
            [t("Export attempts")]: String(r.export.attempts),
            [t("Export result code")]: r.export.last_error_code ?? "—",
            [t("Provider event ID")]: r.export.provider_event_id,
          }).map(([key, value]) => (
            <div key={key}>
              <dt className="text-muted-foreground">{key}</dt>
              <dd className="break-all font-mono">{value}</dd>
            </div>
          ))}
        </dl>
        <h2 className="type-card-title">{t("JavaScript frames")}</h2>
        <ol className="font-mono">
          {r.js_frames.map((f, i) => (
            <li key={i}>
              {f.asset}:{f.line}:{f.column}
            </li>
          ))}
        </ol>
        <h2 className="type-card-title">{t("Native images")}</h2>
        <ol className="font-mono">
          {r.native_images.map((image, i) => (
            <li key={i}>
              {i}: {image.name} · {image.uuid} · {`0x${image.size.toString(16)}`}
            </li>
          ))}
        </ol>
        <h2 className="type-card-title">{t("Native frames")}</h2>
        <ol className="font-mono">
          {r.native_frames.map((f, i) => (
            <li key={i}>
              {f.image ?? "—"} + {`0x${f.offset.toString(16)}`}
            </li>
          ))}
        </ol>
        <p className="text-muted-foreground">
          {t(
            "Frames require the matching private artifacts for symbolication.",
          )}
        </p>
      </CardContent>
    </Card>
  );
}
export function AdminCrashReportPage() {
  useUiTranslation();
  const { id } = useParams({ strict: false });
  const report = useCrashReport(id ?? "");
  return (
    <div className="space-y-6">
      <PageHeader
        title={t("Crash report")}
        actions={
          <Link to="/admin/crash-reports">{t("Back to crash reports")}</Link>
        }
      />
      <LoadState error={report.error} loading={report.isLoading} />
      {report.data && <CrashReportView report={report.data} />}
    </div>
  );
}

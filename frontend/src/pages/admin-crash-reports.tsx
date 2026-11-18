import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { formatDateTime } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import { useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useAppForm } from "@/components/ui/form";
import { useUiTranslation, translateUi as t } from "@/i18n/use-ui-translation";
import {
  useCrashReport,
  useCrashReports,
  useCrashSummary,
  useRegisterCrashRelease,
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
function reportedKinds(counts: Record<string, number>) {
  return Object.entries(counts)
    .filter(([, count]) => count > 0)
    .map(([kind, count]) => `${label(kind, kindLabels)}: ${count}`)
    .join(", ");
}
function LoadState({ error, loading }: { error: unknown; loading: boolean }) {
  return error ? (
    <div role="alert">
      <ErrorBanner message={t("Could not load crash reports.")} />
    </div>
  ) : loading ? (
    <p>{t("Loading…")}</p>
  ) : null;
}
export function AdminCrashReportsPage() {
  useUiTranslation();
  const registerRelease = useRegisterCrashRelease();
  const canRegister = canAdminWrite(useAuthStore((s) => s.user));

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
  const selectVersion = (version: {
    platform: string;
    app_version: string;
    app_build: string;
  }) => {
    form.reset({
      platform: version.platform,
      app_version: version.app_version,
      app_build: version.app_build,
      kind: "",
      fingerprint: "",
    });
    setFilters({
      platform: version.platform,
      app_version: version.app_version,
      app_build: version.app_build,
    });
  };
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
                {label(summary.data.health.rollout, {
                  off: "Rollout off",
                  killed: "Rollout killed",
                  global: "Global rollout",
                  cohort: "Cohort rollout",
                })}
              </p>
              <p>
                {t("Export destinations")}:{" "}
                {summary.data.health.destination_ids.join(", ") || t("None")}
              </p>
              {!summary.data.health.export_enabled && (
                <p role="status">{t("Export disabled")}</p>
              )}
              <section
                aria-labelledby="crash-export-status"
                className="space-y-2"
              >
                <h3 id="crash-export-status" className="type-group-heading">
                  {t("Export status")}
                </h3>
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
              </section>
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>{t("Unverified releases")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <p className="type-secondary text-muted-foreground">
                {t(
                  "Unverified releases are excluded from version counts. Register each store build before release.",
                )}
              </p>
              {registerRelease.isError && (
                <ErrorBanner
                  message={t("Could not register release. Try again.")}
                />
              )}
              <div
                role="list"
                aria-label={t("Unverified releases")}
                className="space-y-3"
              >
                {summary.data.unverified_releases.map((v, i) => (
                  <div
                    role="listitem"
                    key={i}
                    className="rounded-xl border border-border/50 p-4 space-y-2"
                  >
                    <Button
                      variant="link"
                      className="h-auto p-0"
                      onClick={() => selectVersion(v)}
                    >
                      {label(v.platform, platformLabels)} · {v.app_version} (
                      {v.app_build})
                    </Button>
                    <p className="type-secondary">
                      {t("Reported events")}: {v.total} ·{" "}
                      {reportedKinds(v.counts)}
                    </p>
                    <p className="type-secondary break-all">
                      {t("Artifact")}: {v.artifact_ids.join(", ") || "—"}
                    </p>
                    {canRegister &&
                      (v.platform === "web" ? v.artifact_ids : [undefined]).map(
                        (artifact) => (
                          <Button
                            key={artifact ?? "store"}
                            variant="outline"
                            disabled={registerRelease.isPending}
                            onClick={() =>
                              registerRelease.mutate({
                                platform: v.platform,
                                app_version: v.app_version,
                                app_build: v.app_build,
                                ...(artifact ? { artifact_id: artifact } : {}),
                              })
                            }
                          >
                            {t("Register release")}
                            {artifact ? ` · ${artifact}` : ""}
                          </Button>
                        ),
                      )}
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>{t("Reported events by version")}</CardTitle>
            </CardHeader>
            <CardContent>
              <div
                role="list"
                aria-label={t("Reported events by version")}
                className="flex flex-col gap-3 md:hidden"
              >
                {summary.data.versions.map((v, i) => (
                  <div
                    role="listitem"
                    key={i}
                    className="rounded-xl border border-border/50 bg-card p-4"
                  >
                    <Button
                      variant="link"
                      className="h-auto p-0 type-row-title"
                      onClick={() => selectVersion(v)}
                    >
                      {label(v.platform, platformLabels)} · {v.app_version} (
                      {v.app_build})
                    </Button>
                    <dl className="mt-2 space-y-1 type-secondary">
                      {[
                        [t("Environment"), v.environment],
                        [t("Artifact"), v.artifact_ids.join(", ") || "—"],
                        [t("Reported events"), v.total],
                        [t("Reported events by kind"), reportedKinds(v.counts)],
                        [t("Last seen"), formatDateTime(v.last_seen)],
                      ].map(([key, value]) => (
                        <div
                          key={key}
                          className="flex flex-wrap justify-between gap-x-3"
                        >
                          <dt className="text-muted-foreground">{key}</dt>
                          <dd className="break-all">{value}</dd>
                        </div>
                      ))}
                    </dl>
                  </div>
                ))}
              </div>
              <Table
                aria-label={t("Reported events by version")}
                containerClassName="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden"
                className="w-full text-left type-table-cell"
              >
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
                      <TableHead key={h}>{t(h)}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {summary.data.versions.map((v, i) => (
                    <TableRow key={i}>
                      <TableCell>{label(v.platform, platformLabels)}</TableCell>
                      <TableCell>{v.environment}</TableCell>
                      <TableCell>
                        <Button variant="link" onClick={() => selectVersion(v)}>
                          {v.app_version}
                        </Button>
                      </TableCell>
                      <TableCell className="font-mono">{v.app_build}</TableCell>
                      <TableCell className="font-mono">
                        {v.artifact_ids.join(", ") || "—"}
                      </TableCell>
                      <TableCell>{v.total}</TableCell>
                      <TableCell>{reportedKinds(v.counts)}</TableCell>
                      <TableCell className="font-mono">
                        {formatDateTime(v.last_seen)}
                      </TableCell>
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
        <CardContent className="space-y-3">
          <LoadState error={reports.error} loading={reports.isLoading} />
          {reports.data?.reports.length === 0 && (
            <p>{t("No reports in this selection.")}</p>
          )}
          <div
            role="list"
            aria-label={t("Individual reports")}
            className="flex flex-col gap-3 md:hidden"
          >
            {reports.data?.reports.map((r) => (
              <div
                role="listitem"
                key={r.id}
                className="rounded-xl border border-border/50 bg-card p-4"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Link
                    to="/admin/crash-reports/$id"
                    params={{ id: r.id }}
                    className="font-mono type-row-title underline"
                  >
                    {formatDateTime(r.captured_at)}
                  </Link>
                  {r.release_verified === false && (
                    <Badge variant="warning">
                      {t("Unverified release")}
                    </Badge>
                  )}
                </div>
                <dl className="mt-2 space-y-1 type-secondary">
                  {[
                    [t("Platform"), label(r.platform, platformLabels)],
                    [t("Version"), `${r.app_version} (${r.app_build})`],
                    [t("Kind"), label(r.kind, kindLabels)],
                    [t("Export status"), label(r.export.status, exportLabels)],
                  ].map(([key, value]) => (
                    <div
                      key={key}
                      className="flex flex-wrap justify-between gap-x-3"
                    >
                      <dt className="text-muted-foreground">{key}</dt>
                      <dd>{value}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            ))}
          </div>
          <Table
            aria-label={t("Individual reports")}
            containerClassName="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden"
            className="w-full text-left type-table-cell"
          >
            <TableHeader>
              <TableRow>
                {[
                  "Captured",
                  "Platform",
                  "Version",
                  "Kind",
                  "Export status",
                ].map((h) => (
                  <TableHead key={h}>{t(h)}</TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {reports.data?.reports.map((r) => (
                <TableRow key={r.id}>
                  <TableCell>
                    <div className="flex flex-wrap items-center gap-2">
                      <Link
                        to="/admin/crash-reports/$id"
                        params={{ id: r.id }}
                        className="font-mono underline"
                      >
                        {formatDateTime(r.captured_at)}
                      </Link>
                      {r.release_verified === false && (
                        <Badge variant="warning">
                          {t("Unverified release")}
                        </Badge>
                      )}
                    </div>
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
        {r.release_verified === false && (
          <Badge variant="warning">{t("Unverified release")}</Badge>
        )}
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
              {i}: {image.name} · {image.uuid} ·{" "}
              {`0x${image.size.toString(16)}`}
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

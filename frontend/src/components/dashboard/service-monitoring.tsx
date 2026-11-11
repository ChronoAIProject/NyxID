import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Activity, RefreshCw } from "lucide-react";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useMonitoring } from "@/hooks/use-monitoring";
import { ApiError } from "@/lib/api-client";
import {
  displayedCheckStatus,
  monitoringFormDefaults,
  monitoringFormSchema,
  monitoringPayload,
  type MonitoringCheck,
  type MonitoringCheckResult,
  type MonitoringConfig,
  type MonitoringFormValues,
  type MonitoringResult,
} from "@/schemas/monitoring";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

const statusLabels = {
  healthy: "Healthy",
  unhealthy: "Unhealthy",
  ready: "Ready",
  not_ready: "Not ready",
  not_configured: "Not configured",
  unknown: "Unknown",
};
const sourceLabels = {
  owner: "Owner",
  builtin: "Built-in",
  discovered: "Discovered",
  none: "None",
};

function CheckSummary({
  title,
  check,
  now,
  configuration,
}: {
  title: string;
  check: MonitoringCheckResult | null;
  now: number;
  configuration?: MonitoringCheck | null;
}) {
  useUiTranslation();
  const status = check ? displayedCheckStatus(check, now) : "unknown";
  const variant =
    status === "healthy" || status === "ready"
      ? "success"
      : status === "unhealthy" || status === "not_ready"
        ? "destructive"
        : "secondary";
  const checkedAt = check?.checked_at ? Date.parse(check.checked_at) : NaN;
  return (
    <section className="min-w-0 space-y-2" aria-label={title}>
      <div className="flex items-center gap-2">
        <h4 className="type-card-title">{title}</h4>
        <Badge variant={variant}>{statusLabels[status]}</Badge>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-14">
        <dt className="text-muted-foreground">{translateUi("Source")}</dt>
        <dd>
          {
            sourceLabels[
              !check?.checked_at && configuration
                ? "owner"
                : (check?.source ?? "none")
            ]
          }
        </dd>
        <dt className="text-muted-foreground">{translateUi("Path")}</dt>
        <dd className="break-all font-mono">
          {check?.path || configuration?.path || "—"}
        </dd>
        {configuration && (
          <>
            <dt className="text-muted-foreground">{translateUi("Expected HTTP")}</dt>
            <dd>{configuration.expect_status.join(", ")}</dd>
            {configuration.body_contains && (
              <>
                <dt className="text-muted-foreground">{translateUi("Body contains")}</dt>
                <dd className="break-all font-mono">
                  {configuration.body_contains}
                </dd>
              </>
            )}
          </>
        )}
        <dt className="text-muted-foreground">{translateUi("Last checked")}</dt>
        <dd>
          {Number.isFinite(checkedAt)
            ? new Date(checkedAt).toLocaleString(getFormattingLocale())
            : translateUi("Not checked")}
        </dd>
        <dt className="text-muted-foreground">{translateUi("Freshness")}</dt>
        <dd>
          {check?.freshness === "fresh" && status !== "unknown"
            ? translateUi("Fresh")
            : check?.checked_at
              ? translateUi("Stale or unknown")
              : translateUi("Unknown")}
        </dd>
        <dt className="text-muted-foreground">{translateUi("Duration / HTTP")}</dt>
        <dd>
          {Number.isFinite(checkedAt) && check?.duration_ms != null
            ? translateUi("{{value1}} ms", { value1: Math.round(check.duration_ms) })
            : "—"}{" "}
          / {Number.isFinite(checkedAt) ? (check?.status_code ?? "—") : "—"}
        </dd>
        <dt className="text-muted-foreground">{translateUi("Reason")}</dt>
        <dd className="break-words">
          {check?.reason.replaceAll("_", " ") || "—"}
        </dd>
      </dl>
    </section>
  );
}

function CheckFields({
  kind,
  form,
  disabled,
  onEdit,
}: {
  kind: "health" | "readiness";
  form: ReturnType<typeof useAppForm<MonitoringFormValues>>;
  disabled: boolean;
  onEdit: () => void;
}) {
  useUiTranslation();
  const enabled = form.watch(`${kind}.enabled`);
  const errors = form.formState.errors[kind];
  return (
    <fieldset
      disabled={disabled}
      className="space-y-3 rounded-lg border border-border p-3"
    >
      <Label asChild><legend className="px-1">
        {kind === "health" ? translateUi("Health") : translateUi("Readiness")}
      </legend></Label>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={`${kind}-enabled`}>{translateUi("Use owner configuration")}</Label>
        <Switch
          id={`${kind}-enabled`}
          checked={enabled}
          onCheckedChange={(value) => {
            onEdit();
            form.setValue(`${kind}.enabled`, value);
          }}
        />
      </div>
      {enabled && (
        <>
          <div className="space-y-1.5">
            <Label htmlFor={`${kind}-path`}>{translateUi("Relative GET path")}</Label>
            <Input
              id={`${kind}-path`}
              placeholder={kind === "health" ? translateUi("healthz") : translateUi("readyz")}
              {...form.register(`${kind}.path`)}
              aria-invalid={Boolean(errors?.path)}
            />
            {errors?.path && (
              <p role="alert" className="text-14 text-destructive">
                {errors.path.message}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`${kind}-statuses`}>{translateUi("Expected HTTP statuses")}</Label>
            <Input
              id={`${kind}-statuses`}
              placeholder="200, 204"
              {...form.register(`${kind}.statuses`)}
              aria-invalid={Boolean(errors?.statuses)}
            />
            {errors?.statuses && (
              <p role="alert" className="text-14 text-destructive">
                {errors.statuses.message}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`${kind}-body`}>{translateUi("Body contains (optional)")}</Label>
            <Input
              id={`${kind}-body`}
              autoComplete="off"
              {...form.register(`${kind}.body`)}
              aria-invalid={Boolean(errors?.body)}
            />
            {errors?.body && (
              <p role="alert" className="text-14 text-destructive">
                {errors.body.message}
              </p>
            )}
          </div>
        </>
      )}
    </fieldset>
  );
}

function ConfigurationForm({
  config,
  busy,
  canProbe,
  onSave,
  onTest,
  onCancel,
  onEdit,
}: {
  config: MonitoringConfig;
  busy: boolean;
  canProbe: boolean;
  onSave: (values: MonitoringFormValues) => void;
  onTest: (values: MonitoringFormValues) => void;
  onCancel: () => void;
  onEdit: () => void;
}) {
  useUiTranslation();
  const form = useAppForm<MonitoringFormValues>({
    resolver: zodResolver(monitoringFormSchema),
    defaultValues: monitoringFormDefaults(config),
  });
  const hasCheck =
    form.watch("health.enabled") || form.watch("readiness.enabled");
  return (
    <Form {...form}>
      <form
        onSubmit={form.handleSubmit(onSave)}
        onChangeCapture={onEdit}
        className="space-y-4"
      >
        <CheckFields
          kind="health"
          form={form}
          disabled={busy}
          onEdit={onEdit}
        />
        <CheckFields
          kind="readiness"
          form={form}
          disabled={busy}
          onEdit={onEdit}
        />
        <p className="text-14 text-muted-foreground">{translateUi("Health falls back to an available built-in or discovered check when owner configuration is off. Readiness is checked only when explicitly configured.")}</p>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button type="button" onClick={onCancel} disabled={busy}>{translateUi("Cancel")}</Button>
          <Button
            type="button"
            disabled={busy || !canProbe || !hasCheck}
            onClick={() => void form.handleSubmit(onTest)()}
          >{translateUi("Test draft")}</Button>
          <Button
            type="submit"
            variant="primary"
            isLoading={busy}
            disabled={busy || !form.formState.isDirty}
          >{translateUi("Save")}</Button>
        </div>
        <FormSubmitErrors />
      </form>
    </Form>
  );
}

export function ServiceMonitoring({ serviceId }: { serviceId: string }) {
  useUiTranslation();
  const enabled = useFeature(FEATURE_FLAG.SERVICE_MONITORING);
  return enabled ? <MonitoringCard serviceId={serviceId} /> : null;
}

function MonitoringCard({ serviceId }: { serviceId: string }) {
  useUiTranslation();
  const { query, operation } = useMonitoring(serviceId);
  const [editing, setEditing] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [draftResult, setDraftResult] = useState<MonitoringResult | null>(null);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 10_000);
    return () => window.clearInterval(timer);
  }, []);
  const busy = operation.isPending;
  const data = query.isError ? undefined : query.data;
  const config = data?.configuration;
  const result = data?.status;
  const displayNow = Math.max(now, query.dataUpdatedAt);
  const error = operation.error;
  const conflict = error instanceof ApiError && error.status === 409;
  if (
    [query.error, error].some(
      (failure) =>
        failure instanceof ApiError &&
        failure.errorResponse.error === "monitoring_not_configured",
    )
  )
    return null;
  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <CardTitle className="flex items-center gap-2 text-15">
            <Activity className="size-4" />{translateUi("Monitoring")}</CardTitle>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              aria-label={translateUi("Refresh monitoring status")}
              isLoading={query.isFetching}
              disabled={busy || query.isFetching}
              onClick={() => {
                setDraftResult(null);
                void query.refetch();
              }}
            >
              <RefreshCw className="size-3" />{translateUi("Refresh")}</Button>
            <Button
              size="sm"
              disabled={!data?.can_probe || busy}
              isLoading={busy && operation.variables?.kind === "probe"}
              onClick={() => {
                setDraftResult(null);
                operation.mutate({ kind: "probe" });
              }}
            >{translateUi("Check now")}</Button>
            {data?.can_manage && config && (
              <>
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    operation.reset();
                    setDraftResult(null);
                    setEditing(true);
                  }}
                >{translateUi("Configure")}</Button>
                <Button
                  size="sm"
                  disabled={
                    busy ||
                    (!config.owner_config.health &&
                      !config.owner_config.readiness)
                  }
                  onClick={() => {
                    operation.reset();
                    setClearing(true);
                  }}
                >{translateUi("Clear configuration")}</Button>
              </>
            )}
          </div>
        </div>
        <CardDescription>{translateUi("Latest health and readiness checks from chrono-probe. Viewing this section does not run a live check.")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {query.isPending ? (
          <p className="text-14 text-muted-foreground" role="status">{translateUi("Loading monitoring…")}</p>
        ) : query.isError ? (
          <p role="alert" className="text-14 text-muted-foreground">{translateUi("Monitoring is unavailable. Health and readiness are unknown. Refresh to try again.")}</p>
        ) : (
          result && (
            <div className="grid gap-5 sm:grid-cols-2">
              <CheckSummary
                title={translateUi("Health")}
                check={result.health}
                now={displayNow}
                configuration={config?.owner_config.health}
              />
              <CheckSummary
                title={translateUi("Readiness")}
                check={result.readiness}
                now={displayNow}
                configuration={config?.owner_config.readiness}
              />
            </div>
          )
        )}
        {error && !editing && !clearing && (
          <p role="alert" className="text-14 text-destructive">
            {error.message}
          </p>
        )}
        <Dialog
          open={editing}
          onOpenChange={(open) => {
            if (!busy) {
              setEditing(open);
              operation.reset();
            }
          }}
        >
          <DialogContent className="max-h-[90dvh] overflow-y-auto">
            <DialogHeader>
              <DialogTitle>{translateUi("Configure monitoring")}</DialogTitle>
              <DialogDescription>{translateUi("Set safe GET checks for this service. Testing a draft does not save it.")}</DialogDescription>
            </DialogHeader>
            {config && (
              <ConfigurationForm
                key={config.revision}
                config={config}
                busy={busy}
                canProbe={Boolean(data?.can_probe)}
                onEdit={() => setDraftResult(null)}
                onCancel={() => setEditing(false)}
                onSave={(values) =>
                  operation.mutate(
                    {
                      kind: "save",
                      revision: config.revision,
                      checks: monitoringPayload(values),
                    },
                    {
                      onSuccess: () => {
                        setEditing(false);
                        setDraftResult(null);
                      },
                    },
                  )
                }
                onTest={(values) =>
                  operation.mutate(
                    { kind: "validate", checks: monitoringPayload(values) },
                    {
                      onSuccess: (value) => {
                        setNow(Date.now());
                        setDraftResult(value as MonitoringResult);
                      },
                    },
                  )
                }
              />
            )}
            {error && (
              <p role="alert" className="text-14 text-destructive">
                {error.message}
              </p>
            )}
            {conflict && (
              <Button
                disabled={busy}
                onClick={() => {
                  setEditing(false);
                  operation.reset();
                  void query.refetch();
                }}
              >{translateUi("Reload configuration")}</Button>
            )}
            {draftResult && (
              <div className="space-y-2 border-t border-border pt-3">
                <p className="type-row-title">{translateUi("Draft test result (not saved)")}</p>
                <div className="grid gap-4 sm:grid-cols-2">
                  <CheckSummary
                    title={translateUi("Draft health")}
                    check={draftResult.health}
                    now={now}
                  />
                  <CheckSummary
                    title={translateUi("Draft readiness")}
                    check={draftResult.readiness}
                    now={now}
                  />
                </div>
              </div>
            )}
          </DialogContent>
        </Dialog>
        <Dialog
          open={clearing}
          onOpenChange={(open) => {
            if (!busy) {
              setClearing(open);
              operation.reset();
            }
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{translateUi("Clear owner configuration?")}</DialogTitle>
              <DialogDescription>{translateUi("This removes your health and readiness settings. An available built-in or discovered health check may take over.")}</DialogDescription>
            </DialogHeader>
            {error && (
              <p role="alert" className="text-14 text-destructive">
                {error.message}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button disabled={busy} onClick={() => setClearing(false)}>{translateUi("Cancel")}</Button>
              <Button
                variant="destructive"
                isLoading={busy}
                disabled={busy || !config || conflict}
                onClick={() => {
                  if (config)
                    operation.mutate(
                      { kind: "clear", revision: config.revision },
                      {
                        onSuccess: () => {
                          setClearing(false);
                          setDraftResult(null);
                        },
                      },
                    );
                }}
              >{translateUi("Clear configuration")}</Button>
            </div>
            {conflict && (
              <Button
                disabled={busy}
                onClick={() => {
                  setClearing(false);
                  operation.reset();
                  void query.refetch();
                }}
              >{translateUi("Reload configuration")}</Button>
            )}
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  );
}

import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Activity, RefreshCw } from "lucide-react";
import { useMonitoring } from "@/hooks/use-monitoring";
import { ApiError } from "@/lib/api-client";
import {
  displayedCheckStatus,
  monitoringFormDefaults,
  monitoringFormSchema,
  monitoringPayload,
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
}: {
  title: string;
  check: MonitoringCheckResult | null;
  now: number;
}) {
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
        <h4 className="text-13 font-medium">{title}</h4>
        <Badge variant={variant}>{statusLabels[status]}</Badge>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-12">
        <dt className="text-muted-foreground">Source</dt>
        <dd>{sourceLabels[check?.source ?? "none"]}</dd>
        <dt className="text-muted-foreground">Path</dt>
        <dd className="break-all font-mono">{check?.path || "—"}</dd>
        <dt className="text-muted-foreground">Last checked</dt>
        <dd>
          {Number.isFinite(checkedAt)
            ? new Date(checkedAt).toLocaleString()
            : "Not checked"}
        </dd>
        <dt className="text-muted-foreground">Freshness</dt>
        <dd>
          {check?.freshness === "fresh" && status !== "unknown"
            ? "Fresh"
            : check?.checked_at
              ? "Stale or unknown"
              : "Unknown"}
        </dd>
        <dt className="text-muted-foreground">Duration / HTTP</dt>
        <dd>
          {check?.duration_ms != null
            ? `${Math.round(check.duration_ms)} ms`
            : "—"}{" "}
          / {check?.status_code ?? "—"}
        </dd>
        <dt className="text-muted-foreground">Reason</dt>
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
  const enabled = form.watch(`${kind}.enabled`);
  const errors = form.formState.errors[kind];
  return (
    <fieldset
      disabled={disabled}
      className="space-y-3 rounded-lg border border-border p-3"
    >
      <legend className="px-1 text-13 font-medium">
        {kind === "health" ? "Health" : "Readiness"}
      </legend>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={`${kind}-enabled`}>Use owner configuration</Label>
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
            <Label htmlFor={`${kind}-path`}>Relative GET path</Label>
            <Input
              id={`${kind}-path`}
              placeholder={kind === "health" ? "healthz" : "readyz"}
              {...form.register(`${kind}.path`)}
              aria-invalid={Boolean(errors?.path)}
            />
            {errors?.path && (
              <p role="alert" className="text-12 text-destructive">
                {errors.path.message}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`${kind}-statuses`}>Expected HTTP statuses</Label>
            <Input
              id={`${kind}-statuses`}
              placeholder="200, 204"
              {...form.register(`${kind}.statuses`)}
              aria-invalid={Boolean(errors?.statuses)}
            />
            {errors?.statuses && (
              <p role="alert" className="text-12 text-destructive">
                {errors.statuses.message}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={`${kind}-body`}>Body contains (optional)</Label>
            <Input
              id={`${kind}-body`}
              autoComplete="off"
              {...form.register(`${kind}.body`)}
              aria-invalid={Boolean(errors?.body)}
            />
            {errors?.body && (
              <p role="alert" className="text-12 text-destructive">
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
        <p className="text-12 text-muted-foreground">
          Health falls back to an available built-in or discovered check when
          owner configuration is off. Readiness is checked only when explicitly
          configured.
        </p>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button type="button" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={busy || !canProbe || !hasCheck}
            onClick={() => void form.handleSubmit(onTest)()}
          >
            Test draft
          </Button>
          <Button
            type="submit"
            variant="primary"
            isLoading={busy}
            disabled={busy || !form.formState.isDirty}
          >
            Save
          </Button>
        </div>
        <FormSubmitErrors />
      </form>
    </Form>
  );
}

export function ServiceMonitoring({ serviceId }: { serviceId: string }) {
  const { query, operation } = useMonitoring(serviceId);
  const [editing, setEditing] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [manual, setManual] = useState<{
    kind: "probe" | "validate";
    result: MonitoringResult;
  } | null>(null);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 10_000);
    return () => window.clearInterval(timer);
  }, []);
  const busy = operation.isPending;
  const data = query.isError ? undefined : query.data;
  const config = data?.configuration;
  const result = data?.status;
  const error = operation.error;
  const conflict = error instanceof ApiError && error.status === 409;
  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <CardTitle className="flex items-center gap-2 text-15">
            <Activity className="size-4" />
            Monitoring
          </CardTitle>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              aria-label="Refresh monitoring status"
              isLoading={query.isFetching}
              disabled={busy || query.isFetching}
              onClick={() => {
                setManual(null);
                void query.refetch();
              }}
            >
              <RefreshCw className="size-3" />
              Refresh
            </Button>
            <Button
              size="sm"
              disabled={!data?.can_probe || busy}
              isLoading={busy && operation.variables?.kind === "probe"}
              onClick={() => {
                operation.mutate(
                  { kind: "probe" },
                  {
                    onSuccess: (value) => {
                      setNow(Date.now());
                      setManual({
                        kind: "probe",
                        result: value as MonitoringResult,
                      });
                    },
                  },
                );
              }}
            >
              Check now
            </Button>
            {data?.can_manage && config && (
              <>
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    operation.reset();
                    setManual(null);
                    setEditing(true);
                  }}
                >
                  Configure
                </Button>
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
                >
                  Clear configuration
                </Button>
              </>
            )}
          </div>
        </div>
        <CardDescription>
          Health and readiness observed through chrono-probe. Viewing this
          section does not run a live check.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {query.isPending ? (
          <p className="text-12 text-muted-foreground" role="status">
            Loading monitoring…
          </p>
        ) : query.isError ? (
          <p role="alert" className="text-12 text-muted-foreground">
            Monitoring is unavailable. Health and readiness are unknown. Refresh
            to try again.
          </p>
        ) : (
          result && (
            <div className="grid gap-5 sm:grid-cols-2">
              <CheckSummary title="Health" check={result.health} now={now} />
              <CheckSummary
                title="Readiness"
                check={result.readiness}
                now={now}
              />
            </div>
          )
        )}
        {error && !editing && !clearing && (
          <p role="alert" className="text-12 text-destructive">
            {error.message}
          </p>
        )}
        {manual && (
          <div className="space-y-3 border-t border-border pt-4">
            <h4 className="text-13 font-medium">
              {manual.kind === "validate"
                ? "Draft test result (not saved)"
                : "Manual check result"}
            </h4>
            <div className="grid gap-5 sm:grid-cols-2">
              <CheckSummary
                title="Checked health"
                check={manual.result.health}
                now={now}
              />
              <CheckSummary
                title="Checked readiness"
                check={manual.result.readiness}
                now={now}
              />
            </div>
          </div>
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
              <DialogTitle>Configure monitoring</DialogTitle>
              <DialogDescription>
                Set safe GET checks for this service. Testing a draft does not
                save it.
              </DialogDescription>
            </DialogHeader>
            {config && (
              <ConfigurationForm
                key={config.revision}
                config={config}
                busy={busy}
                canProbe={Boolean(data?.can_probe)}
                onEdit={() =>
                  setManual((current) =>
                    current?.kind === "validate" ? null : current,
                  )
                }
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
                        setManual(null);
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
                        setManual({
                          kind: "validate",
                          result: value as MonitoringResult,
                        });
                      },
                    },
                  )
                }
              />
            )}
            {error && (
              <p role="alert" className="text-12 text-destructive">
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
              >
                Reload configuration
              </Button>
            )}
            {manual?.kind === "validate" && (
              <div className="space-y-2 border-t border-border pt-3">
                <p className="text-12 font-medium">
                  Draft test result (not saved)
                </p>
                <div className="grid gap-4 sm:grid-cols-2">
                  <CheckSummary
                    title="Draft health"
                    check={manual.result.health}
                    now={now}
                  />
                  <CheckSummary
                    title="Draft readiness"
                    check={manual.result.readiness}
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
              <DialogTitle>Clear owner configuration?</DialogTitle>
              <DialogDescription>
                This removes your health and readiness settings. An available
                built-in or discovered health check may take over.
              </DialogDescription>
            </DialogHeader>
            {error && (
              <p role="alert" className="text-12 text-destructive">
                {error.message}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button disabled={busy} onClick={() => setClearing(false)}>
                Cancel
              </Button>
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
                          setManual(null);
                        },
                      },
                    );
                }}
              >
                Clear configuration
              </Button>
            </div>
            {conflict && (
              <Button
                disabled={busy}
                onClick={() => {
                  setClearing(false);
                  operation.reset();
                  void query.refetch();
                }}
              >
                Reload configuration
              </Button>
            )}
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  );
}

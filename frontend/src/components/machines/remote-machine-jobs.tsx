import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  readRemoteMachineJobOutput,
  useCancelRemoteMachineJob,
  useRemoteMachineJobs,
} from "@/hooks/use-machines";
import type {
  RemoteMachineJob,
  RemoteMachineJobOutput,
  RemoteMachineSubject,
} from "@/schemas/machines";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const localTime = z
  .string()
  .refine(
    (value) => !value || Number.isFinite(Date.parse(value)),
    "Choose a valid date and time.",
  );
const filtersSchema = z
  .object({ status: z.string(), from: localTime, to: localTime })
  .refine(
    (value) =>
      !value.from ||
      !value.to ||
      Date.parse(value.from) <= Date.parse(value.to),
    { message: "End time must be after start time.", path: ["to"] },
  );
function subjectText(subject: RemoteMachineSubject) {
  if (subject.kind === "assistant")
    return subject.agent_id
      ? translateUi("Assistant Agent {{id}}", { id: subject.agent_id })
      : translateUi("Assistant or legacy principal; Agent identity not reported");
  if (subject.kind === "oauth")
    return translateUi("OAuth client {{id}}", { id: subject.oauth_client_id });
  return translateUi("Parent Key {{id}}", { id: subject.api_key_id }) + (subject.api_key_credential_id ? translateUi(" · Child {{id}}", { id: subject.api_key_credential_id }) : "");
}
const when = (value: string | null) =>
  value ? new Date(value).toLocaleString(getFormattingLocale()) : translateUi("Not reported");
const active = (job: RemoteMachineJob) =>
  ["accepted", "queued", "running", "starting"].includes(job.status);

export function RemoteMachineJobs({
  machineId,
}: {
  readonly machineId: string;
}) {
  const { t } = useUiTranslation();
  const ownerId = useAuthStore((state) => state.user?.id);
  const form = useAppForm<z.infer<typeof filtersSchema>>({
    resolver: zodResolver(filtersSchema),
    defaultValues: { status: "all", from: "", to: "" },
  });
  const [filters, setFilters] = useState<{
    status?: string;
    from?: string;
    to?: string;
  }>({});
  const [cursors, setCursors] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [cancelTarget, setCancelTarget] = useState<RemoteMachineJob | null>(
    null,
  );
  const cancelId = cancelTarget?.id;
  const [cancelNotice, setCancelNotice] = useState<string | null>(null);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const query = useRemoteMachineJobs(machineId, {
    ...filters,
    cursor: cursors.at(-1),
  });
  const cancel = useCancelRemoteMachineJob();
  const jobs = query.data?.jobs ?? [];
  const cancelCurrent =
    !query.error &&
    jobs.some(
      (job) =>
        job.id === cancelTarget?.id &&
        job.runtime_generation === cancelTarget.runtime_generation &&
        job.runtime_id === cancelTarget.runtime_id &&
        job.cancellation_available === true &&
        active(job),
    );
  const selected = jobs.find((job) => job.id === selectedId);
  const missing =
    query.error instanceof ApiError &&
    [404, 405, 501].includes(query.error.status);
  const apply = form.handleSubmit((value) => {
    setFilters({
      status: value.status === "all" ? undefined : value.status,
      from: value.from ? new Date(value.from).toISOString() : undefined,
      to: value.to ? new Date(value.to).toISOString() : undefined,
    });
    setCursors([]);
    setSelectedId(null);
  });
  return (
    <div className="space-y-4">
      <p className="text-12 text-muted-foreground">{t("Find jobs by status and creation time without a known job ID. Metadata remains for 30 days after termination. Commands, input, environment values and full output are not stored in this history.")}</p>
      <Form {...form}>
        <form onSubmit={apply} className="grid gap-3 sm:grid-cols-2">
          <FormField
            control={form.control}
            name="status"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Status")}</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {[
                      "all",
                      "running",
                      "finished",
                      "cancelled",
                      "interrupted",
                      "revoked",
                      "expired",
                    ].map((status) => (
                      <SelectItem key={status} value={status}>
                        {status === "all"
                          ? t("All statuses")
                          : status.replaceAll("_", " ")}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
          {(["from", "to"] as const).map((name) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    {name === "from" ? t("Created from") : t("Created to")}
                  </FormLabel>
                  <FormControl>
                    <Input type="datetime-local" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          ))}
          <div className="flex items-end gap-2">
            <Button type="submit" disabled={!form.formState.isDirty}>{t("Apply filters")}</Button>
            <Button type="button" onClick={() => void query.refetch()}>{t("Refresh")}</Button>
          </div>
        </form>
      </Form>
      <p className="text-12 text-muted-foreground">{t("Date filters use your local timezone and are sent as absolute timestamps.")}</p>
      {cancelNotice && (
        <p role="status" className="text-12 text-muted-foreground">
          {cancelNotice}
        </p>
      )}
      {query.isPending && (
        <p role="status" className="text-12">{t("Loading jobs…")}</p>
      )}
      {query.error &&
        (missing ? (
          <p
            role="status"
            className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground"
          >{t("Job history is unavailable on this server. This does not mean the machine has no jobs. The owner job-list API must be connected before status, cancellation, and output can be reviewed here.")}</p>
        ) : (
          <ErrorBanner
            message={t("Job history could not be loaded. {{message}}", { message: translateErrorMessage(query.error.message) })}
            onRetry={() => void query.refetch()}
          />
        ))}
      {!query.isPending && !query.error && !jobs.length && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">{t("No jobs match these filters.")}</p>
      )}
      <div className="flex flex-col gap-3 md:hidden">
        {jobs.map((job) => (
          <article
            key={job.id}
            className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
          >
            <p className="break-all font-mono text-13 font-semibold">
              {job.id}
            </p>
            <div className="flex flex-wrap gap-2">
              <Badge variant={active(job) ? "warning" : "secondary"}>
                {job.status}
              </Badge>
              <Badge variant="secondary">{t("Output ")}{job.output_availability}
              </Badge>
            </div>
            <p className="break-all text-11 text-muted-foreground">
              {subjectText(job.subject)}
            </p>
            <p className="text-11 text-text-tertiary">
              {when(job.started_at ?? job.created_at)}{t(" · Generation")}{" "}
              {job.runtime_generation ?? t("Not reported")}
            </p>
            <Button onClick={() => setSelectedId(job.id)}>{t("View job")}</Button>
          </article>
        ))}
      </div>
      {!!jobs.length && (
        <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("Job / principal")}</TableHead>
                <TableHead>{t("Status / generation")}</TableHead>
                <TableHead>{t("Time")}</TableHead>
                <TableHead>{t("Output")}</TableHead>
                <TableHead>{t("Details")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.map((job) => (
                <TableRow key={job.id}>
                  <TableCell>
                    <p className="break-all font-mono text-11">{job.id}</p>
                    <p className="break-all text-11 text-muted-foreground">
                      {subjectText(job.subject)}
                    </p>
                  </TableCell>
                  <TableCell>
                    {job.status}
                    <p className="text-11 text-text-tertiary">{t("Generation ")}{job.runtime_generation ?? t("Not reported")}
                      {job.exit_code !== null ? t(" · Exit {{exit_code}}", { exit_code: job.exit_code }) : ""}
                    </p>
                  </TableCell>
                  <TableCell>
                    {when(job.started_at ?? job.created_at)}
                  </TableCell>
                  <TableCell>{job.output_availability}</TableCell>
                  <TableCell>
                    <Button onClick={() => setSelectedId(job.id)}>{t("View job")}</Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <div className="flex justify-between gap-2">
        <Button
          disabled={!cursors.length}
          onClick={() => {
            setCursors((values) => values.slice(0, -1));
            setSelectedId(null);
          }}
        >{t("Previous jobs")}</Button>
        <Button
          disabled={!query.data?.next_cursor || query.isFetching}
          onClick={() => {
            const next = query.data?.next_cursor;
            if (next) setCursors((values) => [...values, next]);
            setSelectedId(null);
          }}
        >{t("Next jobs")}</Button>
      </div>
      {selected && !query.error && (
        <div className="space-y-4">
          <DetailSection
            title={t("Selected job")}
            action={
              <Button variant="ghost" onClick={() => setSelectedId(null)}>{t("Close job")}</Button>
            }
          >
            <DetailRow label={t("Job ID")} value={selected.id} mono copyable />
            <DetailRow
              label={t("Principal")}
              value={subjectText(selected.subject)}
              mono
            />
            <DetailRow
              label={t("Acting person")}
              value={selected.subject.user_id}
              mono
            />
            <DetailRow
              label={t("Grant")}
              value={selected.grant_id ?? "Assistant assignment"}
              mono
            />
            <DetailRow label={t("Status")} value={selected.status} />
            <DetailRow label={t("Created")} value={when(selected.created_at)} />
            <DetailRow label={t("Started")} value={when(selected.started_at)} />
            <DetailRow
              label={t("Duration")}
              value={
                selected.duration_ms == null
                  ? "Not reported"
                  : `${selected.duration_ms} ms`
              }
            />
            <DetailRow
              label={t("History retained until")}
              value={when(selected.retain_until ?? null)}
            />
            <DetailRow
              label={t("Cancellation requested")}
              value={when(selected.cancellation_requested_at ?? null)}
            />
            <DetailRow
              label={t("Cancellation requester")}
              value={
                selected.cancellation_requested_by?.kind === "human_owner"
                  ? `Human owner ${selected.cancellation_requested_by.user_id}`
                  : selected.cancellation_requested_by?.kind === "external"
                    ? subjectText(selected.cancellation_requested_by.principal)
                    : selected.cancellation_requested_by?.kind === "system"
                      ? `System: ${selected.cancellation_requested_by.reason}`
                      : "Not reported"
              }
            />
            <DetailRow label={t("Finished")} value={when(selected.finished_at)} />
            <DetailRow
              label={t("Authority expires")}
              value={when(selected.expires_at)}
            />
            <DetailRow
              label={t("Exit code")}
              value={
                selected.exit_code === null
                  ? "Not reported"
                  : String(selected.exit_code)
              }
            />
            <DetailRow
              label={t("Stop / cancellation reason")}
              value={selected.stop_reason ?? "Not reported"}
            />
            <DetailRow
              label={t("Runtime generation")}
              value={
                selected.runtime_generation === null
                  ? "Not reported"
                  : String(selected.runtime_generation)
              }
            />
            <DetailRow
              label={t("Declared services")}
              value={
                selected.services.map((service) => service.slug).join(", ") ||
                "None"
              }
            />
            <DetailRow
              label={t("Output availability")}
              value={selected.output_availability}
            />
            <DetailRow
              label={t("Output deadline")}
              value={when(selected.output_expires_at)}
            />
            {selected.output_reason && (
              <p className="px-4 py-3 text-12 text-muted-foreground">
                {selected.output_reason}
              </p>
            )}
          </DetailSection>
          <MachineOutput
            key={`${ownerId}:${machineId}:${selected.id}:${selected.runtime_id}:${selected.runtime_generation}:${selected.output_query_available}:${selected.output_availability}:${selected.output_expires_at}`}
            machineId={machineId}
            job={selected}
          />
          {active(selected) && selected.cancellation_available !== true && (
            <p className="text-12 text-muted-foreground">{t("Owner cancellation is not exposed by this server. No cancellation request can be sent here.")}</p>
          )}
          {active(selected) && (
            <Button
              disabled={selected.cancellation_available !== true}
              variant="destructive"
              onClick={() => setCancelTarget(selected)}
            >{t("Cancel job")}</Button>
          )}
        </div>
      )}
      <Dialog
        open={Boolean(cancelId)}
        onOpenChange={(open) => {
          if (!open) {
            setCancelTarget(null);
            setCancelError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Cancel job")}</DialogTitle>
            <DialogDescription>{t("Request cancellation of this job’s process group. Cancellation does not resume the machine or extend execution authority.")}</DialogDescription>
          </DialogHeader>
          {cancelError && <ErrorBanner message={cancelError} />}
          <DialogFooter>
            <Button
              onClick={() => setCancelTarget(null)}
              disabled={cancel.isPending}
            >{t("Keep running")}</Button>
            <Button
              variant="destructive"
              isLoading={cancel.isPending}
              disabled={!cancelCurrent}
              onClick={async () => {
                if (
                  !cancelId ||
                  !cancelCurrent ||
                  useAuthStore.getState().user?.id !== ownerId
                )
                  return;
                try {
                  const result = await cancel.mutateAsync({
                    machineId,
                    jobId: cancelId,
                  });
                  if (useAuthStore.getState().user?.id !== ownerId) return;
                  setCancelNotice(
                    result.cancellation_acknowledged
                      ? "Cancellation request acknowledged. Physical stop is not confirmed by this response; refresh job metadata for the eventual outcome."
                      : "Cancellation was requested. Physical stop is not confirmed by this response; refresh job metadata for the eventual outcome.",
                  );
                  setCancelTarget(null);
                  setCancelError(null);
                } catch (error) {
                  setCancelError(
                    error instanceof Error
                      ? error.message
                      : "Cancellation could not be requested.",
                  );
                }
              }}
            >{t("Confirm cancellation")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export function MachineOutput({
  machineId,
  job,
}: {
  readonly machineId: string;
  readonly job: RemoteMachineJob;
}) {
  const { t } = useUiTranslation();
  const ownerId = useAuthStore((state) => state.user?.id);
  const [output, setOutput] = useState<RemoteMachineJobOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const request = useRef<AbortController | null>(null);
  const expires = job.output_expires_at
    ? Date.parse(job.output_expires_at)
    : null;
  const [expired, setExpired] = useState(
    expires !== null && expires <= Date.now(),
  );
  const available =
    (job.output_availability === "available" ||
      (job.output_availability === "query_required" &&
        job.output_query_available === true)) &&
    !expired;
  useEffect(() => {
    return () => {
      request.current?.abort();
    };
  }, []);
  useEffect(() => {
    if (expires === null) return;
    const timer = window.setInterval(() => {
      if (expires <= Date.now()) {
        request.current?.abort();
        setOutput(null);
        setExpired(true);
      }
    }, 1000);
    return () => window.clearInterval(timer);
  }, [expires]);
  async function load(next = false) {
    if (!available || !ownerId) return;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setLoading(true);
    setError(null);
    setOutput(null);
    try {
      const page = await readRemoteMachineJobOutput(
        machineId,
        job.id,
        next && output
          ? {
              ...(output.output_offset === undefined
                ? {}
                : { output_offset: output.output_offset }),
              ...(output.stderr_offset === undefined
                ? {}
                : { stderr_offset: output.stderr_offset }),
            }
          : {},
        controller.signal,
      );
      if (
        !controller.signal.aborted &&
        useAuthStore.getState().user?.id === ownerId
      )
        setOutput(page);
    } catch (cause) {
      if (!controller.signal.aborted)
        setError(
          cause instanceof ApiError && [404, 409, 410].includes(cause.status)
            ? "Output is no longer available from the live node. Expired, evicted, or lost output cannot be recovered here."
            : cause instanceof Error
              ? cause.message
              : "Output could not be read.",
        );
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }
  return (
    <DetailSection title={t("Live job output")}>
      <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
        <p>{t("Output is fetched on demand from the live authorized node and held only in this open view. It is never copied into Query caches, browser storage, or history. It may disappear before the one-hour maximum after job completion.")}</p>
        {!available && (
          <p role="status">{t("Output ")}{expired ? t("expired") : job.output_availability}{t(". Lost or expired output cannot be recovered from job history.")}</p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!available || loading}
            isLoading={loading}
            onClick={() => void load()}
          >{t("Read live output")}</Button>
          {output && output.output_available !== false && (
            <>
              <Button
                onClick={() => {
                  request.current?.abort();
                  setOutput(null);
                }}
              >{t("Clear output")}</Button>
              <Button
                disabled={
                  loading ||
                  output.output_offset === undefined ||
                  output.stderr_offset === undefined ||
                  (!output.truncated && !active(job))
                }
                onClick={() => void load(true)}
              >{t("Next output page")}</Button>
            </>
          )}
        </div>
        {output?.output_available === false && (
          <p role="status">{t("Output unavailable:")}{" "}
            {(output.output_unavailable_reason ?? t("not reported")).replaceAll(
              "_",
              " ",
            )}{t(". Expired or evicted bytes cannot be recovered from history.")}</p>
        )}
        {error && (
          <div role="alert">
            <ErrorBanner message={error} />
          </div>
        )}
        {output && output.output_available !== false && (
          <>
            <p className="text-11">{t("Status ")}{output.status}{t(" · Duration")}{" "}
              {output.duration_ms === undefined
                ? t("not reported")
                : t("{{duration_ms}} ms", { duration_ms: output.duration_ms })}{" "}{t("· Exit ")}{output.exit_code ?? t("pending")}
              {output.timed_out ? t(" · Timed out") : ""}
              {output.truncated
                ? t(" · Output was truncated; earlier bytes may have been discarded")
                : ""}
            </p>
            <h4 className="font-medium text-foreground">{t("Standard output")}</h4>
            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border bg-muted p-3 font-mono text-11">
              {output.stdout === undefined
                ? "(not reported)"
                : output.stdout || "(empty)"}
            </pre>
            <h4 className="font-medium text-foreground">{t("Standard error")}</h4>
            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border bg-muted p-3 font-mono text-11">
              {output.stderr === undefined
                ? "(not reported)"
                : output.stderr || "(empty)"}
            </pre>
          </>
        )}
      </div>
    </DetailSection>
  );
}

import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { describeChanges } from "@/lib/grouped-form-changes";
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
import {
  GroupedInput as Input,
  GroupedSelectTrigger as SelectTrigger,
  GroupedList,
  GroupedRow,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedLabel as FormLabel,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { Form, FormControl, FormField, useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
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
  return `Parent Key ${subject.api_key_id}${subject.api_key_credential_id ? translateUi(" · Child {{id}}", { id: subject.api_key_credential_id }) : ""}`;
}
const when = (value: string | null) =>
  value ? new Date(value).toLocaleString(getFormattingLocale()) : "Not reported";
const active = (job: RemoteMachineJob) =>
  ["accepted", "queued", "running", "starting"].includes(job.status);

export function RemoteMachineJobs({
  machineId,
}: {
  readonly machineId: string;
}) {
  useUiTranslation();
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
    form.reset(value);
  });
  return (
    <div className="space-y-4">
      <p className="text-14 text-muted-foreground">{translateUi("Find jobs by status and creation time without a known job ID. Metadata remains for 30 days after termination. Commands, input, environment values and full output are not stored in this history.")}</p>
      <Form {...form}>
        <form onSubmit={apply} className="space-y-6">
          <GroupedList
            title={translateUi("Filter jobs")}
            footer={translateUi("Date filters use your local timezone and are sent as absolute timestamps. Finished includes all recorded exit results, including nonzero exit codes. A cancelled record alone does not confirm a physical stop.")}
          >
            <FormField
              control={form.control}
              name="status"
              render={({ field }) => (
                <FormItem inline>
                  <FormLabel>{translateUi("Status")}</FormLabel>
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
                            ? translateUi("All statuses")
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
                      {name === "from" ? translateUi("Created from") : translateUi("Created to")}
                    </FormLabel>
                    <FormControl>
                      <Input type="datetime-local" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            ))}
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void query.refetch()}>{translateUi("Refresh")}</button>
            </GroupedRow>
          </GroupedList>
          <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "status", label: "Status", format: (value) => String(value).replace(/^./, (letter) => letter.toUpperCase()) }, { key: "from", label: "From" }, { key: "to", label: "To" }])}
            label={translateUi("job filters")}
            dirty={form.formState.isDirty}
            pending={query.isFetching}
            save={apply}
            discard={() => form.reset()}
            saveLabel={translateUi("Apply filters")}
          />
        </form>
      </Form>
      {cancelNotice && (
        <p role="status" className="text-14 text-muted-foreground">
          {cancelNotice}
        </p>
      )}
      {query.isPending && (
        <p role="status" className="text-14">{translateUi("Loading jobs…")}</p>
      )}
      {query.error &&
        (missing ? (
          <p
            role="status"
            className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground"
          >{translateUi("Job history is unavailable on this server. This does not mean the machine has no jobs. The owner job-list API must be connected before status, cancellation, and output can be reviewed here.")}</p>
        ) : (
          <ErrorBanner
            message={translateUi("Job history could not be loaded. {{value1}}", { value1: query.error.message })}
            onRetry={() => void query.refetch()}
          />
        ))}
      {!query.isPending && !query.error && !jobs.length && (
        <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">{translateUi("No jobs match these filters.")}</p>
      )}
      {!query.isPending && !query.error && (
        <p className="text-14 text-muted-foreground">{translateUi("Loaded ")}{jobs.length} {jobs.length === 1 ? translateUi("job") : translateUi("jobs")}{translateUi(" on this page.")}</p>
      )}
      <div className="flex flex-col gap-3 md:hidden">
        {jobs.map((job) => (
          <article
            key={job.id}
            className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
          >
            <p className="break-all font-mono type-card-title">
              {job.id}
            </p>
            <div className="flex flex-wrap gap-2">
              <Badge variant={active(job) ? "warning" : "secondary"}>
                {job.status}
              </Badge>
              <Badge variant="secondary">{translateUi("Output ")}{job.output_availability}
              </Badge>
            </div>
            <p className="break-all text-13 text-muted-foreground">
              {subjectText(job.subject)}
            </p>
            <p className="text-13 text-text-tertiary">
              {when(job.started_at ?? job.created_at)}{translateUi(" · Generation")}{" "}
              {job.runtime_generation ?? translateUi("Not reported")}
            </p>
            {job.status === "finished" && (
              <p className="text-13 text-muted-foreground">
                {job.exit_code === null
                  ? translateUi("Exit result not reported")
                  : translateUi("Exit code {{value1}}", { value1: job.exit_code })}
              </p>
            )}
            <Button onClick={() => setSelectedId(job.id)}>{translateUi("View job")}</Button>
          </article>
        ))}
      </div>
      {!!jobs.length && (
        <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{translateUi("Job / principal")}</TableHead>
                <TableHead>{translateUi("Status / generation")}</TableHead>
                <TableHead>{translateUi("Time")}</TableHead>
                <TableHead>{translateUi("Output")}</TableHead>
                <TableHead>{translateUi("Details")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.map((job) => (
                <TableRow key={job.id}>
                  <TableCell>
                    <p className="break-all font-mono text-13">{job.id}</p>
                    <p className="break-all text-13 text-muted-foreground">
                      {subjectText(job.subject)}
                    </p>
                  </TableCell>
                  <TableCell>
                    {job.status}
                    <p className="text-13 text-text-tertiary">{translateUi("Generation ")}{job.runtime_generation ?? translateUi("Not reported")}
                      {job.exit_code !== null ? translateUi(" · Exit {{value1}}", { value1: job.exit_code }) : ""}
                    </p>
                  </TableCell>
                  <TableCell>
                    {when(job.started_at ?? job.created_at)}
                  </TableCell>
                  <TableCell>{job.output_availability}</TableCell>
                  <TableCell>
                    <Button onClick={() => setSelectedId(job.id)}>{translateUi("View job")}</Button>
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
        >{translateUi("Previous jobs")}</Button>
        <Button
          disabled={!query.data?.next_cursor || query.isFetching}
          onClick={() => {
            const next = query.data?.next_cursor;
            if (next) setCursors((values) => [...values, next]);
            setSelectedId(null);
          }}
        >{translateUi("Next jobs")}</Button>
      </div>
      {selected && !query.error && (
        <div className="space-y-4">
          <DetailSection
            title={translateUi("Selected job")}
            action={
              <Button variant="ghost" onClick={() => setSelectedId(null)}>{translateUi("Close job")}</Button>
            }
          >
            <DetailRow label={translateUi("Job ID")} value={selected.id} mono copyable />
            <DetailRow
              label={translateUi("Principal")}
              value={subjectText(selected.subject)}
              mono
            />
            <DetailRow
              label={translateUi("Acting person")}
              value={selected.subject.user_id}
              mono
            />
            <DetailRow
              label={translateUi("Grant")}
              value={selected.grant_id ?? "Assistant assignment"}
              mono
            />
            <DetailRow label={translateUi("Status")} value={selected.status} />
            <DetailRow label={translateUi("Created")} value={when(selected.created_at)} />
            <DetailRow label={translateUi("Started")} value={when(selected.started_at)} />
            <DetailRow
              label={translateUi("Duration")}
              value={
                selected.duration_ms == null
                  ? "Not reported"
                  : `${selected.duration_ms} ms`
              }
            />
            <DetailRow
              label={translateUi("History retained until")}
              value={when(selected.retain_until ?? null)}
            />
            <DetailRow
              label={translateUi("Cancellation requested")}
              value={when(selected.cancellation_requested_at ?? null)}
            />
            <DetailRow
              label={translateUi("Cancellation requester")}
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
            <DetailRow label={translateUi("Finished")} value={when(selected.finished_at)} />
            <DetailRow
              label={translateUi("Authority expires")}
              value={when(selected.expires_at)}
            />
            <DetailRow
              label={translateUi("Exit code")}
              value={
                selected.exit_code === null
                  ? "Not reported"
                  : String(selected.exit_code)
              }
            />
            <DetailRow
              label={translateUi("Stop / cancellation reason")}
              value={selected.stop_reason ?? "Not reported"}
            />
            <DetailRow
              label={translateUi("Runtime generation")}
              value={
                selected.runtime_generation === null
                  ? "Not reported"
                  : String(selected.runtime_generation)
              }
            />
            <DetailRow
              label={translateUi("Declared services")}
              value={
                selected.services.map((service) => service.slug).join(", ") ||
                "None"
              }
            />
            <DetailRow
              label={translateUi("Output availability")}
              value={selected.output_availability}
            />
            <DetailRow
              label={translateUi("Output deadline")}
              value={when(selected.output_expires_at)}
            />
            {selected.output_reason && (
              <p className="px-4 py-3 text-14 text-muted-foreground">
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
            <p className="text-14 text-muted-foreground">{translateUi("Owner cancellation is not exposed by this server. No cancellation request can be sent here.")}</p>
          )}
          {active(selected) && (
            <Button
              disabled={selected.cancellation_available !== true}
              variant="destructive"
              onClick={() => setCancelTarget(selected)}
            >{translateUi("Cancel job")}</Button>
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
            <DialogTitle>{translateUi("Cancel job")}</DialogTitle>
            <DialogDescription>{translateUi("Request cancellation of this job’s process group. Cancellation does not resume the machine or extend execution authority.")}</DialogDescription>
          </DialogHeader>
          {cancelError && <ErrorBanner message={cancelError} />}
          <DialogFooter>
            <Button
              onClick={() => setCancelTarget(null)}
              disabled={cancel.isPending}
            >{translateUi("Keep running")}</Button>
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
            >{translateUi("Confirm cancellation")}</Button>
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
  useUiTranslation();
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
    <DetailSection title={translateUi("Live job output")}>
      <div className="space-y-3 px-4 py-3 text-14 text-muted-foreground">
        <p>{translateUi("Output is fetched on demand from the live authorized node and held only in this open view. It is never copied into Query caches, browser storage, or history. It may disappear before the one-hour maximum after job completion.")}</p>
        {!available && (
          <p role="status">{translateUi("Output ")}{expired ? translateUi("expired") : job.output_availability}. Lost or
            expired output cannot be recovered from job history.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!available || loading}
            isLoading={loading}
            onClick={() => void load()}
          >{translateUi("Read live output")}</Button>
          {output && output.output_available !== false && (
            <>
              <Button
                onClick={() => {
                  request.current?.abort();
                  setOutput(null);
                }}
              >{translateUi("Clear output")}</Button>
              <Button
                disabled={
                  loading ||
                  output.output_offset === undefined ||
                  output.stderr_offset === undefined ||
                  (!output.truncated && !active(job))
                }
                onClick={() => void load(true)}
              >{translateUi("Next output page")}</Button>
            </>
          )}
        </div>
        {output?.output_available === false && (
          <p role="status">{translateUi("Output unavailable:")}{" "}
            {(output.output_unavailable_reason ?? "not reported").replaceAll(
              "_",
              " ",
            )}
            . Expired or evicted bytes cannot be recovered from history.
          </p>
        )}
        {error && (
          <div role="alert">
            <ErrorBanner message={error} />
          </div>
        )}
        {output && output.output_available !== false && (
          <>
            <p className="text-13">{translateUi("Status ")}{output.status}{translateUi(" · Duration")}{" "}
              {output.duration_ms === undefined
                ? translateUi("not reported")
                : translateUi("{{value1}} ms", { value1: output.duration_ms })}{" "}{translateUi("· Exit ")}{output.exit_code ?? translateUi("pending")}
              {output.timed_out ? translateUi(" · Timed out") : ""}
              {output.truncated
                ? translateUi(" · Output was truncated; earlier bytes may have been discarded")
                : ""}
            </p>
            <h4 className="font-medium text-foreground">{translateUi("Standard output")}</h4>
            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border bg-muted p-3 font-mono text-14">
              {output.stdout === undefined
                ? "(not reported)"
                : output.stdout || "(empty)"}
            </pre>
            <h4 className="font-medium text-foreground">{translateUi("Standard error")}</h4>
            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border bg-muted p-3 font-mono text-14">
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

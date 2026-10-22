import { useAuthStore } from "@/stores/auth-store";
import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useDecideRemoteMachineConfirmation,
  useRemoteMachineConfirmations,
} from "@/hooks/use-machines";
import type { RemoteMachineConfirmation } from "@/schemas/machines";
import { ApiError } from "@/lib/api-client";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const confirmationReviewSchema = z.object({
  reviewed: z
    .boolean()
    .refine(
      Boolean,
      "Review the identity and safe parameters before deciding.",
    ),
});
function identity(row: RemoteMachineConfirmation) {
  const subject = row.subject;
  if (subject.kind === "assistant")
    return subject.agent_id
      ? `Assistant Agent ${subject.agent_id}`
      : "Assistant Agent identity not reported";
  if (subject.kind === "oauth")
    return `OAuth client ${subject.oauth_client_id}`;
  return `Parent Key ${subject.api_key_id}${subject.api_key_credential_id ? ` · Child ${subject.api_key_credential_id}` : ""}`;
}
function reviewable(row: RemoteMachineConfirmation, now: number) {
  return (
    row.status === "pending" &&
    row.review_parameters_available === true &&
    row.safe_parameters.length > 0 &&
    row.safe_parameters.every(
      (parameter) => parameter.name.trim() && parameter.value.trim(),
    ) &&
    row.runtime_generation !== null &&
    Number.isSafeInteger(row.runtime_generation) &&
    Date.parse(row.expires_at) > now
  );
}

/** Compare every displayed authority/review field, not just the digest. */
function reviewSnapshot(row: RemoteMachineConfirmation) {
  return JSON.stringify({
    id: row.id,
    node_id: row.node_id,
    subject: row.subject,
    grant_id: row.grant_id,
    grant_revision: row.grant_revision,
    assignment_revision: row.assignment_revision,
    runtime_id: row.runtime_id,
    runtime_generation: row.runtime_generation,
    operation: row.operation,
    parameters_digest: row.parameters_digest,
    safe_parameters: row.safe_parameters,
    review_parameters_available: row.review_parameters_available,
    review_content_available: row.review_content_available,
    status: row.status,
    created_at: row.created_at,
    expires_at: row.expires_at,
  });
}
export function RemoteMachineConfirmations({
  machineId,
  confirmationId,
}: {
  readonly machineId: string;
  readonly confirmationId?: string;
}) {
  const owner = useAuthStore((state) => state.user?.id);
  const context = `${owner ?? ""}:${machineId}`;
  const [pages, setPages] = useState<{
    machineId: string;
    cursors: (string | undefined)[];
  }>({ machineId: context, cursors: [undefined] });
  if (pages.machineId !== context)
    setPages({ machineId: context, cursors: [undefined] });
  const cursor = pages.machineId === context ? pages.cursors.at(-1) : undefined;
  const query = useRemoteMachineConfirmations(machineId, cursor);
  const [review, setReview] = useState<RemoteMachineConfirmation | null>(null);
  const [notice, setNotice] = useState<{
    context: string;
    message: string;
  } | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const missing =
    query.error instanceof ApiError &&
    [404, 405, 501].includes(query.error.status);
  const rows = query.data?.confirmations ?? [];
  const linked = rows.find((row) => row.id === confirmationId);
  const reviewStillCurrent = Boolean(
    review &&
    !query.error &&
    !query.isPending &&
    !query.isFetching &&
    rows.some(
      (row) =>
        row.node_id === machineId &&
        reviewable(row, clock) &&
        reviewSnapshot(row) === reviewSnapshot(review),
    ),
  );
  // Discard the selection immediately; a later refresh must require a new review.
  if (review && !reviewStillCurrent) setReview(null);
  return (
    <div className="space-y-4">
      {notice?.context === context && (
        <p
          role="status"
          className="rounded-lg bg-overlay px-4 py-3 text-12 text-muted-foreground"
        >
          {notice.message}
        </p>
      )}
      <p className="text-12 text-muted-foreground">
        Only the human owner can approve external execution. The client must
        retry the same operation and parameters using the same valid identity;
        approval is consumed once after live authorization is checked again.
        Approval does not execute the request from this page.
      </p>
      <p className="text-12 text-muted-foreground">
        Waiting approval expires after 15 minutes. Once approved, execution has
        a separate window of at most 10 minutes and remains limited by grant,
        credential, runtime and funding deadlines. A different child credential
        cannot consume this approval.
      </p>
      <p className="text-12 text-muted-foreground">
        Each page contains at most 100 live pending or approved confirmations.
        {query.data?.next_cursor === undefined
          ? " Older-page availability is unknown on this server."
          : " Older pages use the server’s stable cursor."}
      </p>
      <p className="rounded-lg bg-overlay px-4 py-3 text-12 text-muted-foreground">
        Assistant action confirmations stay in the original conversation. This
        list contains external client requests, not Assistant conversation
        cards.
      </p>
      {query.isPending && (
        <p role="status" className="text-12">
          Loading confirmations…
        </p>
      )}
      {query.error &&
        (missing ? (
          <p
            role="status"
            className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground"
          >
            External confirmations are unavailable on this server. Requests that
            require confirmation must remain blocked; this page cannot approve
            or bypass them until the owner confirmation API is connected.
          </p>
        ) : (
          <ErrorBanner
            message={`Confirmations could not be loaded. ${query.error.message}`}
            onRetry={() => void query.refetch()}
          />
        ))}
      {confirmationId && !linked && !query.isPending && !query.error && (
        <p role="status" className="text-12 text-muted-foreground">
          The linked confirmation is not on this page. Check older pages; it may
          have expired or no longer be visible under your current authorization.
        </p>
      )}
      {!query.isPending && !query.error && !rows.length && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
          No pending or approved confirmations are available.
        </p>
      )}
      {rows.map((row) => {
        const expired =
          row.status === "pending" && Date.parse(row.expires_at) <= clock;
        const approvedExpired =
          row.status === "approved" &&
          row.approved_until !== null &&
          Date.parse(row.approved_until) <= clock;
        const status = expired || approvedExpired ? "expired" : row.status;
        return (
          <DetailSection
            key={row.id}
            title={row.operation}
            action={
              <Badge
                variant={
                  status === "pending"
                    ? "warning"
                    : status === "approved"
                      ? "success"
                      : "secondary"
                }
              >
                {status}
              </Badge>
            }
          >
            <DetailRow label="Confirmation ID" value={row.id} mono />
            <DetailRow label="Principal" value={identity(row)} mono />
            <DetailRow label="Acting person" value={row.subject.user_id} mono />
            <DetailRow
              label="Grant / assignment revision"
              value={`${row.grant_revision} / ${row.assignment_revision}`}
            />
            <DetailRow
              label="Runtime generation"
              value={
                row.runtime_generation === null
                  ? "Unknown"
                  : String(row.runtime_generation)
              }
            />
            <DetailRow
              label={
                row.status === "approved"
                  ? "Approved retry deadline"
                  : "Waiting approval expires"
              }
              value={new Date(row.expires_at).toLocaleString()}
            />
            {row.approved_until && row.status !== "approved" && (
              <DetailRow
                label="Approved retry deadline"
                value={new Date(row.approved_until).toLocaleString()}
              />
            )}
            <div className="space-y-2 px-4 py-3 text-12 text-muted-foreground">
              {status === "approved" && (
                <p role="status">
                  Approved — waiting for the original caller to retry within the
                  returned deadline. This approval does not execute the request;
                  live permissions and Machine state are checked again.
                </p>
              )}
              {row.safe_parameters.length ? (
                <dl className="space-y-2">
                  {row.safe_parameters.map((parameter, index) => (
                    <div key={`${index}:${parameter.name}`}>
                      <dt className="font-medium text-foreground">
                        {parameter.name}
                      </dt>
                      <dd className="break-words">{parameter.value}</dd>
                    </div>
                  ))}
                </dl>
              ) : (
                <p>
                  No safe parameter summary was provided. Approval is
                  unavailable until the server provides a reviewable summary.
                </p>
              )}
              {!reviewable(row, clock) && status === "pending" && (
                <p role="status">
                  A server-confirmed safe summary and known runtime generation
                  are required before either approval or decline is available
                  here.
                </p>
              )}
              <p>
                Only server-redacted parameters appear here. Raw commands,
                environment values, input and secrets are not shown.
              </p>
              <Button
                disabled={
                  Boolean(query.error) ||
                  query.isPending ||
                  query.isFetching ||
                  row.node_id !== machineId ||
                  !reviewable(row, clock)
                }
                onClick={() => setReview(row)}
              >
                Review decision
              </Button>
            </div>
          </DetailSection>
        );
      })}
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={pages.cursors.length < 2 || query.isFetching}
          onClick={() =>
            setPages({
              machineId: context,
              cursors: pages.cursors.slice(0, -1),
            })
          }
        >
          Newer confirmations
        </Button>
        <Button
          disabled={!query.data?.next_cursor || query.isFetching}
          onClick={() => {
            if (query.data?.next_cursor)
              setPages({
                machineId: context,
                cursors: [...pages.cursors, query.data.next_cursor],
              });
          }}
        >
          Older confirmations
        </Button>
        <Button
          onClick={() => {
            setPages({ machineId: context, cursors: [undefined] });
            void query.refetch();
          }}
        >
          Refresh confirmations
        </Button>
      </div>
      {review && reviewStillCurrent && (
        <ConfirmationDecision
          key={review.id}
          confirmation={review}
          machineId={machineId}
          onClose={(message) => {
            if (useAuthStore.getState().user?.id !== owner) return;
            setReview(null);
            setPages({ machineId: context, cursors: [undefined] });
            if (message) setNotice({ context, message });
          }}
        />
      )}
    </div>
  );
}
export function ConfirmationDecision({
  confirmation,
  machineId,
  onClose,
}: {
  readonly confirmation: RemoteMachineConfirmation;
  readonly machineId: string;
  readonly onClose: (message?: string) => void;
}) {
  const mutation = useDecideRemoteMachineConfirmation();
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const form = useAppForm<z.infer<typeof confirmationReviewSchema>>({
    resolver: zodResolver(confirmationReviewSchema),
    defaultValues: { reviewed: false },
  });
  const [error, setError] = useState<string | null>(null);
  const [expired, setExpired] = useState(
    () => Date.parse(confirmation.expires_at) <= Date.now(),
  );
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (Date.parse(confirmation.expires_at) <= Date.now()) setExpired(true);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [confirmation.expires_at]);
  async function decide(approve: boolean) {
    if (
      mutation.isPending ||
      expired ||
      error ||
      confirmation.node_id !== machineId ||
      !reviewable(confirmation, Date.now()) ||
      confirmation.runtime_generation === null
    )
      return;
    if (approve && !(await form.trigger())) return;
    // Recheck after asynchronous form validation so expiry never crosses dispatch.
    if (
      !active.current ||
      !reviewable(confirmation, Date.now()) ||
      confirmation.runtime_generation === null
    )
      return;
    try {
      const result = await mutation.mutateAsync({
        machineId,
        confirmationId: confirmation.id,
        body: {
          approve,
          expected_grant_revision: confirmation.grant_revision,
          expected_runtime_generation: confirmation.runtime_generation,
          parameters_digest: confirmation.parameters_digest,
        },
      });
      onClose(
        result.status === "approved"
          ? "Approval recorded. Waiting for the original caller to retry within the returned deadline; the request has not been executed by this page."
          : result.status === "declined"
            ? "Request declined. This page has not executed the operation."
            : "Decision response received. Refresh confirmations to check the latest status.",
      );
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Decision could not be recorded. Refresh before reviewing again.",
      );
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !mutation.isPending) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Review external request</DialogTitle>
          <DialogDescription>
            This decision is bound to the displayed principal, child, grant,
            runtime generation and parameter digest.
          </DialogDescription>
        </DialogHeader>
        <p className="break-all text-12 text-muted-foreground">
          {identity(confirmation)}
        </p>
        <p className="text-12 text-muted-foreground">
          Operation: {confirmation.operation}
        </p>
        <DetailSection title="Reviewed authority">
          <DetailRow label="Confirmation ID" value={confirmation.id} mono />
          <DetailRow label="Grant ID" value={confirmation.grant_id} mono />
          <DetailRow
            label="Grant / assignment revision"
            value={`${confirmation.grant_revision} / ${confirmation.assignment_revision}`}
          />
          <DetailRow
            label="Runtime generation"
            value={
              confirmation.runtime_generation === null
                ? "Unknown"
                : String(confirmation.runtime_generation)
            }
          />
          <DetailRow
            label="Parameter digest"
            value={confirmation.parameters_digest}
            mono
          />
          <DetailRow
            label="Waiting approval expires"
            value={new Date(confirmation.expires_at).toLocaleString()}
          />
        </DetailSection>
        {confirmation.review_content_available === false && (
          <p className="text-12 text-muted-foreground">
            Full request content is unavailable. Review the server’s safe
            summary below; it does not verify every effect of the operation.
          </p>
        )}
        <dl className="space-y-2 text-12">
          {confirmation.safe_parameters.map((parameter, index) => (
            <div key={`${index}:${parameter.name}`}>
              <dt className="font-medium">{parameter.name}</dt>
              <dd className="break-words text-muted-foreground">
                {parameter.value}
              </dd>
            </div>
          ))}
        </dl>
        <Form {...form}>
          <FormField
            control={form.control}
            name="reviewed"
            render={({ field }) => (
              <FormItem>
                <div className="flex items-start gap-2">
                  <FormControl>
                    <Checkbox
                      checked={field.value}
                      onCheckedChange={(value) =>
                        field.onChange(value === true)
                      }
                    />
                  </FormControl>
                  <FormLabel className="text-12">
                    I reviewed this identity and safe request summary.
                  </FormLabel>
                </div>
                <FormMessage />
              </FormItem>
            )}
          />
        </Form>
        {expired && (
          <p role="status" className="text-12 text-destructive">
            This confirmation expired. The client must submit a new request.
          </p>
        )}
        {error && (
          <div role="alert">
            <ErrorBanner message={error} />
          </div>
        )}
        <DialogFooter>
          <Button onClick={() => onClose()} disabled={mutation.isPending}>
            Close
          </Button>
          <Button
            variant="destructive"
            disabled={
              expired ||
              mutation.isPending ||
              Boolean(error) ||
              confirmation.node_id !== machineId ||
              !reviewable(confirmation, 0)
            }
            isLoading={mutation.isPending}
            onClick={() => void decide(false)}
          >
            Decline
          </Button>
          <Button
            variant="primary"
            disabled={
              expired ||
              mutation.isPending ||
              Boolean(error) ||
              !form.watch("reviewed") ||
              confirmation.node_id !== machineId ||
              !reviewable(confirmation, 0)
            }
            isLoading={mutation.isPending}
            onClick={() => void decide(true)}
          >
            Approve one retry
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

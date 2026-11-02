import { useEffect, useRef, useState } from "react";
import { Check, Loader2, RefreshCw, TriangleAlert } from "lucide-react";
import { useRetryRemoteMachineDelete } from "@/hooks/use-machines";
import { ApiError } from "@/lib/api-client";
import {
  clearRemoteMachineIntent,
  getRemoteMachineIntent,
  readRemoteMachineIntent,
  type RemoteMachineIntent,
} from "@/lib/remote-machine-intent";
import {
  remoteMachineDeleteRetryRequestSchema,
  type RemoteMachine,
  type RemoteMachineOperation,
} from "@/schemas/machines";
import { useAuthStore } from "@/stores/auth-store";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export interface MachineAccountingView {
  readonly label: string;
  readonly detail: string;
  readonly closed: boolean;
}

function readable(value: string) {
  return value.replaceAll("_", " ");
}

function time(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Not reported";
}

function pending(operation: RemoteMachineOperation) {
  return (
    !operation.terminal_at &&
    ["accepted", "applying", "reconciling"].includes(operation.state)
  );
}

function currentDelete(
  machine: RemoteMachine | undefined,
  operation: RemoteMachineOperation,
) {
  return (
    machine?.resource_state === "deleting" &&
    machine.active_operation_id === operation.id &&
    machine.node_id === operation.machine_id &&
    operation.action === "delete" &&
    pending(operation)
  );
}

/** Presentation of existing observations only. Applying is not stage evidence. */
export function RemoteMachineProgress({
  machine,
  operation,
  acceptedAction,
  readError,
  onCheck,
  onActivity,
  accounting,
}: {
  readonly machine: RemoteMachine;
  readonly operation?: RemoteMachineOperation;
  readonly acceptedAction?: "pause" | "resume" | "delete" | null;
  readonly readError?: string;
  readonly onCheck: () => void | Promise<boolean>;
  readonly onActivity?: () => void;
  readonly accounting?: MachineAccountingView;
}) {
  const [accepted, setAccepted] = useState<{
    before: string;
    operation: RemoteMachineOperation;
  } | null>(null);
  const snapshot = JSON.stringify(operation);
  const current =
    accepted && accepted.before === snapshot ? accepted.operation : operation;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!current?.deadline_at || Date.parse(current.deadline_at) <= now) return;
    const timer = window.setTimeout(
      () => setNow(Date.now()),
      Math.min(
        Math.max(0, Date.parse(current.deadline_at) - Date.now() + 1),
        2_147_483_647,
      ),
    );
    return () => window.clearTimeout(timer);
  }, [current?.deadline_at, now]);
  const action =
    current?.action ??
    acceptedAction ??
    (machine.resource_state === "provisioning"
      ? "create"
      : machine.resource_state === "stopping"
        ? "pause"
        : machine.resource_state === "deleting"
          ? "delete"
          : machine.resource_state === "resuming"
            ? "resume"
            : "reconcile");
  const stopped = Boolean(
    current && currentDelete(machine, current) && current.delete_retry_stopped,
  );
  const failed = current?.state === "failed";
  const delayed =
    !failed &&
    !stopped &&
    (current?.state === "reconciling" ||
      machine.resource_state === "reconciling" ||
      Boolean(current?.deadline_at && Date.parse(current.deadline_at) <= now));
  const confirmed = Boolean(current || acceptedAction);
  const steps =
    action === "create"
      ? ["Request accepted", "Preparing workspace", "Starting machine"]
      : action === "pause"
        ? [
            "Pause requested",
            "Stopping active tasks",
            "Confirming paused state",
          ]
        : action === "delete"
          ? ["Deletion requested", "Stopping runtime", "Removing workspace"]
          : action === "resume"
            ? [
                "Resume requested",
                "Starting runtime",
                "Waiting for machine to connect",
              ]
            : [
                "Request accepted",
                "Checking runtime evidence",
                "Confirming machine state",
              ];
  const title = stopped
    ? "We couldn't finish deleting this machine"
    : failed
      ? `We couldn't ${action === "create" ? "set up" : action} this machine`
      : readError
        ? "Status could not be confirmed"
        : delayed
          ? "Taking longer than usual"
          : action === "create"
            ? "Setting up your machine"
            : action === "pause"
              ? "Pausing your machine"
              : action === "delete"
                ? "Deleting your machine"
                : action === "resume"
                  ? "Resuming your machine"
                  : "Checking your machine";
  const attention = stopped || failed || delayed || Boolean(readError);
  return (
    <section
      aria-label={stopped ? "Deletion recovery" : "Operation progress"}
      aria-live="polite"
      className={`rounded-xl border bg-card p-4 ${attention ? "border-warning/40" : "border-border"}`}
    >
      <div className="flex items-start gap-3">
        {attention ? (
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
        ) : (
          <Loader2 className="mt-0.5 size-4 shrink-0 animate-spin text-text-tertiary" />
        )}
        <div className="min-w-0 flex-1 space-y-1">
          <h3 className="text-13 font-semibold">{title}</h3>
          <p className="text-12 leading-relaxed text-muted-foreground">
            {stopped
              ? "Automatic deletion attempts stopped. Retry deletion to continue. Cleanup and accounting remain pending."
              : readError
                ? readError
                : failed
                  ? action === "resume"
                    ? "Your workspace is retained. Review the current machine state before choosing an available recovery action."
                    : "Review the confirmed result and current machine state before continuing."
                  : delayed
                    ? action === "delete"
                      ? "Deletion is still pending. We'll keep trying automatically while the current round is active."
                      : "We're still waiting for the runtime to confirm. Your workspace is retained."
                    : action === "delete"
                      ? "Deletion is in progress. Waiting for cleanup to be confirmed."
                      : confirmed
                        ? "Your request was accepted. Waiting for confirmation from the machine."
                        : "Reading the current operation. Request acceptance has not yet been confirmed."}
          </p>
          {current?.error_message && (
            <p
              role={failed ? "alert" : "status"}
              className="break-words text-12 text-muted-foreground"
            >
              {current.error_message}
            </p>
          )}
        </div>
      </div>
      {!failed && !stopped && (
        <ol className="mt-4 space-y-3 pl-0.5">
          {steps.map((step, index) => {
            const done = index === 0 && confirmed;
            const active = index === (confirmed ? 1 : 0);
            return (
              <li
                key={step}
                data-state={done ? "confirmed" : active ? "active" : "pending"}
                className="flex items-center gap-2.5 text-12"
              >
                <span
                  aria-label={
                    done
                      ? "Confirmed"
                      : active
                        ? "In progress"
                        : "Awaiting confirmation"
                  }
                  className={`flex size-4 shrink-0 items-center justify-center rounded-full border ${done ? "border-success/30 bg-success/10 text-success" : "border-border text-text-tertiary"}`}
                >
                  {done ? (
                    <Check className="size-2.5" />
                  ) : active ? (
                    <span className="size-1.5 rounded-full bg-current" />
                  ) : null}
                </span>
                <span
                  className={active ? "font-medium" : "text-muted-foreground"}
                >
                  {step}
                </span>
              </li>
            );
          })}
        </ol>
      )}
      {stopped && current && (
        <div className="mt-4">
          <RemoteMachineDeleteRetry
            key={current.id}
            machine={machine}
            operation={current}
            onAccepted={(result) => {
              setAccepted({ before: snapshot, operation: result });
              void onCheck();
            }}
            onReview={async () => Boolean(await onCheck())}
          />
        </div>
      )}
      {accepted && !stopped && (
        <p role="status" className="mt-3 text-12">
          Deletion retry request accepted for the original operation. Deletion
          completion is still awaiting platform observation.
        </p>
      )}
      {action === "delete" && machine.cleanup_state === "complete" && (
        <p className="mt-3 text-12">
          Platform object cleanup confirmed. Final deletion and runtime
          accounting are independently checked.
        </p>
      )}
      <div className="mt-4 flex flex-wrap items-center justify-between gap-2 border-t border-border/50 pt-3">
        <p className="text-11 text-text-tertiary">
          You can close this panel and return later.
        </p>
        {attention && !stopped ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void onCheck()}
          >
            Check status
          </Button>
        ) : onActivity ? (
          <Button type="button" size="sm" variant="ghost" onClick={onActivity}>
            View activity
          </Button>
        ) : null}
      </div>
      {action === "delete" &&
        (machine.free_slot ?? machine.billing_state === "free") && (
          <p className="mt-2 text-11 text-muted-foreground">
            Your free slot stays occupied until deletion is confirmed.
          </p>
        )}
      {current && (
        <details className="mt-3 text-12">
          <summary className="cursor-pointer text-muted-foreground">
            Operation details
          </summary>
          <RemoteMachineOperationCard
            operation={current}
            accounting={accounting}
            diagnosticsOnly
          />
        </details>
      )}
    </section>
  );
}

/** History never receives a Machine binding, so it cannot renew deletion. */
export function RemoteMachineOperationCard({
  operation: observedOperation,
  machine,
  accounting,
  onRetryAccepted,
  onReview,
  diagnosticsOnly = false,
}: {
  readonly operation: RemoteMachineOperation;
  readonly machine?: RemoteMachine;
  readonly accounting?: MachineAccountingView;
  readonly onRetryAccepted?: () => void;
  readonly onReview?: () => Promise<boolean>;
  readonly diagnosticsOnly?: boolean;
}) {
  // The mutation response is authoritative while the read cache still contains
  // the exact pre-submit snapshot. Any subsequent changed read takes over.
  const [accepted, setAccepted] = useState<{
    before: string;
    operation: RemoteMachineOperation;
  } | null>(null);
  const snapshot = JSON.stringify(observedOperation);
  const operation =
    accepted?.before === snapshot ? accepted.operation : observedOperation;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!operation.deadline_at || Date.parse(operation.deadline_at) <= now)
      return;
    const timer = window.setTimeout(
      () => setNow(Date.now()),
      Math.min(
        Math.max(0, Date.parse(operation.deadline_at) - Date.now() + 1),
        2_147_483_647,
      ),
    );
    return () => window.clearTimeout(timer);
  }, [operation.deadline_at, now]);
  const deleting = currentDelete(machine, operation);
  const stopped =
    operation.action === "delete" &&
    pending(operation) &&
    machine?.resource_state !== "deleted" &&
    operation.delete_retry_stopped;
  const overdue =
    operation.action === "delete" &&
    pending(operation) &&
    machine?.resource_state !== "deleted" &&
    operation.deadline_at &&
    Date.parse(operation.deadline_at) < now;
  return (
    <DetailSection
      title={`${readable(operation.action)} operation`}
      className="text-12 max-sm:[&_div.flex.text-12]:flex-col max-sm:[&_div.flex.text-12]:items-start max-sm:[&_div.flex.text-12]:gap-1 max-sm:[&_div.flex.text-12>div]:max-w-full max-sm:[&_.text-right]:text-left"
      action={
        <Badge
          variant={
            operation.state === "succeeded"
              ? "success"
              : operation.state === "failed"
                ? "destructive"
                : "warning"
          }
        >
          {readable(operation.state)}
        </Badge>
      }
    >
      {overdue && (
        <p role="status" className="px-4 py-3 text-12 text-warning">
          Cleanup target exceeded. Deletion remains pending; this time target
          does not identify the cause or guarantee completion.
        </p>
      )}
      {!diagnosticsOnly && operation.action === "delete" && (
        <div className="space-y-2 px-4 py-3 text-12">
          {stopped ? (
            <p role="status">
              We couldn't finish deleting this machine. Automatic deletion
              attempts stopped. Retry deletion to continue. Cleanup and
              accounting remain pending.
            </p>
          ) : pending(operation) ? (
            <p>
              Deletion is still pending. This machine can't be used until
              cleanup is complete.
              {(machine?.free_slot ?? machine?.billing_state === "free") &&
                " Your free slot stays occupied until deletion is confirmed."}
            </p>
          ) : null}
          {deleting && (
            <RemoteMachineDeleteRetry
              key={operation.id}
              machine={machine!}
              operation={operation}
              onAccepted={(result) => {
                setAccepted({ before: snapshot, operation: result });
                onRetryAccepted?.();
              }}
              onReview={onReview}
            />
          )}
        </div>
      )}
      {!diagnosticsOnly && operation.error_message && (
        <p role="alert" className="break-words px-4 py-3 text-destructive">
          {operation.error_message}
        </p>
      )}
      {operation.error_code === "delete_kubernetes_access_denied" && (
        <p className="px-4 py-3 text-muted-foreground">
          The platform must review its Kubernetes cleanup permissions. Retrying
          does not guarantee this access problem will be resolved.
        </p>
      )}
      <details className="text-12">
        <summary className="cursor-pointer px-4 py-3 font-medium">
          View operation progress
        </summary>
        <dl className="grid gap-3 px-4 py-3 sm:grid-cols-3">
          {operation.action === "delete" && (
            <div className="min-w-0 space-y-1">
              <dt className="text-11 text-muted-foreground">
                Attempts this round
              </dt>
              <dd className="font-medium tabular-nums">
                {operation.delete_retry_attempts ?? "Not reported"} /{" "}
                {operation.delete_retry_limit ?? "Not reported"}
              </dd>
            </div>
          )}
          <div className="min-w-0 space-y-1">
            <dt className="text-11 text-muted-foreground">
              {operation.action === "delete"
                ? "Total deletion attempts"
                : "Attempts after retry"}
            </dt>
            <dd className="font-medium tabular-nums">
              {operation.retry_count}
            </dd>
          </div>
          {operation.action === "delete" && (
            <div className="min-w-0 space-y-1">
              <dt className="text-11 text-muted-foreground">
                Next platform attempt
              </dt>
              <dd className="break-words font-medium">
                {operation.next_attempt_at
                  ? time(operation.next_attempt_at)
                  : stopped
                    ? "No automatic attempt scheduled"
                    : "Not reported"}
              </dd>
            </div>
          )}
        </dl>
        <DetailRow
          label="Recovery code"
          value={operation.error_code ?? "No specific cause reported"}
          mono={Boolean(operation.error_code)}
        />
        {machine && (
          <>
            <DetailRow
              label="Platform object cleanup"
              value={readable(machine.cleanup_state)}
            />
            {accounting && (
              <DetailRow label="Runtime accounting" value={accounting.label} />
            )}
            <DetailRow
              label="Final resource result"
              value={readable(machine.resource_state)}
            />
          </>
        )}
        <p className="px-4 py-3 text-muted-foreground">
          Each result is confirmed independently. Elapsed time is not a
          completion percentage. Delete completion confirms Sandbox and related
          namespace objects, terminal creation responsibilities and accounting;
          physical disk release or erasure is not verified.
        </p>
        <details>
          <summary className="cursor-pointer px-4 py-3 font-medium">
            Diagnostic information
          </summary>
          <DetailRow label="Operation ID" value={operation.id} mono copyable />
          <DetailRow
            label="Operation cleanup projection"
            value={readable(operation.cleanup_state)}
          />
          <DetailRow label="Created" value={time(operation.created_at)} />
          <DetailRow label="Updated" value={time(operation.updated_at)} />
          <DetailRow
            label="Cleanup target"
            value={time(operation.deadline_at)}
          />
          {operation.terminal_at && (
            <DetailRow
              label="Terminal result recorded"
              value={time(operation.terminal_at)}
            />
          )}
          {operation.evidence_digest && (
            <DetailRow
              label="Evidence reference"
              value={operation.evidence_digest}
              mono
            />
          )}
        </details>
      </details>
    </DetailSection>
  );
}

type RetryPhase =
  | "idle"
  | "submitting"
  | "accepted"
  | "failed"
  | "unknown"
  | "conflict"
  | "reviewing";

function RemoteMachineDeleteRetry({
  machine,
  operation,
  onAccepted,
  onReview,
}: {
  readonly machine: RemoteMachine;
  readonly operation: RemoteMachineOperation;
  readonly onAccepted: (operation: RemoteMachineOperation) => void;
  readonly onReview?: () => Promise<boolean>;
}) {
  const owner = useAuthStore((state) => state.user?.id ?? "");
  const mutation = useRetryRemoteMachineDelete();
  const scope = `${operation.id}:retry-delete`;
  const [restored] = useState(() => {
    try {
      const intent = readRemoteMachineIntent(owner, scope);
      return { request: intent ? { scope, intent } : null, error: null };
    } catch (cause) {
      return {
        request: null,
        error:
          cause instanceof Error
            ? cause.message
            : "The saved deletion request could not be read.",
      };
    }
  });
  const [phase, setPhase] = useState<RetryPhase>(
    restored.error ? "conflict" : restored.request ? "unknown" : "idle",
  );
  const [error, setError] = useState<string | null>(restored.error);
  const saved = useRef<{ scope: string; intent: RemoteMachineIntent } | null>(
    restored.request,
  );
  const inFlight = useRef(false);
  const [acceptedCount, setAcceptedCount] = useState<number | null>(null);
  const newExhaustedRound =
    phase === "accepted" &&
    operation.delete_retry_stopped &&
    acceptedCount !== null &&
    operation.retry_count > acceptedCount;
  const canSubmit =
    currentDelete(machine, operation) &&
    (operation.delete_retry_stopped || phase === "unknown") &&
    (phase !== "accepted" || newExhaustedRound);
  const busy =
    mutation.isPending || phase === "submitting" || phase === "reviewing";

  async function retry() {
    if (inFlight.current || busy || !canSubmit || phase === "conflict") return;
    let request: { scope: string; intent: RemoteMachineIntent };
    try {
      request = saved.current ?? {
        scope,
        intent: getRemoteMachineIntent(owner, scope, {
          expected_revision: machine.revision,
          expected_retry_count: operation.retry_count,
        }),
      };
      remoteMachineDeleteRetryRequestSchema.parse(request.intent.body);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The deletion retry request could not be prepared.",
      );
      setPhase("failed");
      return;
    }
    saved.current = request;
    inFlight.current = true;
    setError(null);
    setPhase("submitting");
    try {
      const result = await mutation.mutateAsync({
        machineId: machine.node_id,
        operationId: operation.id,
        body: remoteMachineDeleteRetryRequestSchema.parse(request.intent.body),
        idempotencyKey: request.intent.idempotencyKey,
      });
      if (useAuthStore.getState().user?.id !== owner) return;
      // The hook also validates this binding. Keep the view conservative when
      // its mutation adapter cannot confirm the original operation identity.
      if (
        result.operation.id !== operation.id ||
        result.operation.machine_id !== machine.node_id ||
        result.operation.action !== "delete"
      )
        throw new Error(
          "Deletion retry returned a different operation or Machine",
        );
      clearRemoteMachineIntent(
        owner,
        request.scope,
        request.intent.idempotencyKey,
      );
      saved.current = null;
      setAcceptedCount(result.operation.retry_count);
      setPhase("accepted");
      onAccepted(result.operation);
    } catch (cause) {
      if (useAuthStore.getState().user?.id !== owner) return;
      const conflict = cause instanceof ApiError && cause.status === 409;
      const definite =
        cause instanceof ApiError &&
        cause.status >= 400 &&
        cause.status < 500 &&
        cause.status !== 408;
      if (definite) {
        clearRemoteMachineIntent(
          owner,
          request.scope,
          request.intent.idempotencyKey,
        );
        saved.current = null;
      }
      setPhase(conflict ? "conflict" : definite ? "failed" : "unknown");
      setError(
        cause instanceof Error
          ? cause.message
          : "Deletion retry could not be confirmed.",
      );
    } finally {
      inFlight.current = false;
    }
  }

  async function review() {
    if (!onReview || inFlight.current || busy) return;
    setPhase("reviewing");
    const loaded = await onReview();
    if (useAuthStore.getState().user?.id !== owner) return;
    if (loaded) {
      if (restored.error) {
        clearRemoteMachineIntent(owner, scope);
      }
      setError(null);
      setPhase("idle");
    } else {
      setError(
        "The latest machine and operation could not both be read. Refresh again before reviewing deletion.",
      );
      setPhase("conflict");
    }
  }

  return (
    <div className="space-y-2">
      {(canSubmit || busy || phase === "conflict") && (
        <Button
          type="button"
          onClick={() => void (phase === "unknown" ? review() : retry())}
          disabled={
            busy || phase === "conflict" || (phase === "unknown" && !onReview)
          }
          isLoading={busy}
        >
          <RefreshCw className="size-3" />
          {busy
            ? "Retrying deletion…"
            : phase === "unknown"
              ? "Check retry status"
              : "Retry deletion"}
        </Button>
      )}
      {phase === "accepted" && !newExhaustedRound && (
        <p role="status">
          Deletion retry request accepted for the original operation. Deletion
          completion is still awaiting platform observation.
        </p>
      )}
      {phase === "unknown" && (
        <p role="status">
          Deletion retry result unknown. Your request may have been accepted.
          Check its status to continue. The machine has not been marked deleted.
        </p>
      )}
      {phase === "conflict" && (
        <p role="status">
          Deletion changed during review. Refresh both records and review their
          latest state before making a new request.
        </p>
      )}
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      {phase === "conflict" && onReview && (
        <Button type="button" onClick={() => void review()} disabled={busy}>
          Refresh and review deletion
        </Button>
      )}
      {phase === "accepted" &&
        operation.delete_retry_stopped &&
        !newExhaustedRound &&
        onReview && (
          <Button type="button" onClick={() => void review()}>
            Review latest deletion
          </Button>
        )}
    </div>
  );
}

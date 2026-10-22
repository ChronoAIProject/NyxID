import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
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

/** History never receives a Machine binding, so it cannot renew deletion. */
export function RemoteMachineOperationCard({
  operation: observedOperation,
  machine,
  accounting,
  onRetryAccepted,
  onReview,
}: {
  readonly operation: RemoteMachineOperation;
  readonly machine?: RemoteMachine;
  readonly accounting?: MachineAccountingView;
  readonly onRetryAccepted?: () => void;
  readonly onReview?: () => Promise<boolean>;
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
      {operation.action === "delete" && (
        <div className="space-y-2 px-4 py-3 text-12">
          {stopped ? (
            <p role="status">
              Automatic deletion attempts stopped. The current round’s server
              limit is {operation.delete_retry_limit ?? "not reported"}. Cleanup
              and accounting retain their responsibilities.
            </p>
          ) : pending(operation) ? (
            <p>
              Deletion is still pending. Acceptance and scheduled attempts do
              not confirm cleanup or accounting completion.
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
          <dd className="font-medium tabular-nums">{operation.retry_count}</dd>
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
      {operation.error_message && (
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
      const legacyScope = `${scope}:${operation.retry_count}`;
      const intent = readRemoteMachineIntent(owner, scope);
      const legacy = intent
        ? null
        : readRemoteMachineIntent(owner, legacyScope);
      return {
        request: intent
          ? { scope, intent }
          : legacy
            ? { scope: legacyScope, intent: legacy }
            : null,
        error: null,
      };
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
        clearRemoteMachineIntent(owner, `${scope}:${operation.retry_count}`);
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
          onClick={() => void retry()}
          disabled={busy || phase === "conflict"}
          isLoading={busy}
        >
          <RefreshCw className="size-3" />
          {busy
            ? "Retrying deletion…"
            : phase === "unknown"
              ? "Confirm retry result"
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
          Deletion retry result unknown. Confirming the result reuses the same
          request and idempotency key; the machine has not been marked deleted.
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
        <Button onClick={() => void review()} disabled={busy}>
          Refresh and review deletion
        </Button>
      )}
      {phase === "accepted" &&
        operation.delete_retry_stopped &&
        !newExhaustedRound &&
        onReview && (
          <Button onClick={() => void review()}>Review latest deletion</Button>
        )}
    </div>
  );
}

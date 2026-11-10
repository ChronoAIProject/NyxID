import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { Circle, Clock3, ExternalLink, Loader2 } from "lucide-react";
import { safeCmaUrl } from "@/lib/assistant/cma-url";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  isCmaOperationPending,
  type CmaOperation,
} from "@/schemas/cma-operation";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { CmaAgentPanel } from "./cma-agent-panel";
import { CmaCardDetails, CmaCardFrame } from "./cma-card-frame";

function presentation(operation: CmaOperation) {
  const { state, reason, status, kind } = operation;
  if (state === "cancelled")
    return { label: "Tracking stopped", tone: "muted" };
  if (state === "cancelling")
    return { label: "Stopping tracking", tone: "muted" };
  if (reason === "timeout")
    return { label: "Tracking timed out", tone: "warning" };
  if (
    kind === "cma_response" &&
    reason === "operation_cancelled" &&
    status === "incomplete"
  ) {
    return { label: "Cancelled", tone: "muted" };
  }
  if (reason === "submission_failed")
    return { label: "Submission failed", tone: "error" };
  if (reason === "submission_uncertain") {
    return { label: "Submission unconfirmed", tone: "warning" };
  }
  if (
    reason === "operation_failed" ||
    ["failed", "incomplete", "cancelled", "stopped", "deleted"].includes(
      status ?? "",
    )
  ) {
    const label =
      status === "incomplete"
        ? "Incomplete"
        : status === "cancelled" || status === "stopped"
          ? "Stopped"
          : status === "deleted"
            ? "Deleted"
            : kind === "cma_response"
              ? "Failed"
              : "Needs attention";
    return {
      label,
      tone: ["cancelled", "stopped", "deleted"].includes(status ?? "")
        ? "muted"
        : "error",
    };
  }
  const succeeded =
    status ===
    (kind === "cma_response"
      ? "completed"
      : kind === "cma_provisioning"
        ? "succeeded"
        : "ready");
  if (
    reason &&
    reason !== "completed" &&
    !(reason.startsWith("result_") && succeeded)
  ) {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (succeeded) {
    return {
      label: kind === "cma_response" ? "Completed" : "Ready",
      tone:
        kind === "cma_agent" && isCmaOperationPending(operation)
          ? "neutral"
          : "success",
    };
  }
  if (
    reason === "completed" ||
    ["completed", "succeeded", "ready"].includes(status ?? "")
  ) {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (
    [
      "waiting_approval",
      "connection_required",
      "awaiting_connection",
      "waiting_for_connection",
    ].includes(status ?? "")
  ) {
    return {
      label:
        status === "waiting_approval" ? "Needs approval" : "Needs connection",
      tone: "warning",
    };
  }
  if (status === "paused") return { label: "Paused", tone: "muted" };
  if (state === "submitting") return { label: "Submitting", tone: "active" };
  if (!status) return { label: "Awaiting status", tone: "active" };
  if (status === "queued") return { label: "Queued", tone: "active" };
  if (["starting", "provisioning"].includes(status))
    return { label: "Starting", tone: "active" };
  if (["working", "running", "in_progress"].includes(status))
    return {
      label: kind === "cma_provisioning" ? "Starting" : "Working",
      tone: "active",
    };
  return { label: "Status unavailable", tone: "warning" };
}
const tones: Record<string, string> = {
  active: "text-primary-text",
  success: "text-success",
  warning: "text-warning",
  error: "text-destructive",
  muted: "text-text-tertiary",
  neutral: "text-text-tertiary",
};

const phases: Record<string, string> = {
  sandbox_creation: "Creating sandbox",
  sandbox_startup: "Starting sandbox",
  sandbox_ready: "Preparing sandbox",
  workspace_creation: "Preparing workspace",
  agent_creation: "Creating agent",
  agent_startup: "Starting agent",
  desktop_preparation: "Preparing desktop",
};

export function CmaOperationCard({
  operation,
}: {
  readonly operation: CmaOperation;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const detailsId = useId();
  const scrollBody = useRef<HTMLDivElement>(null);
  const cmaUrl = safeCmaUrl(operation.cma_url);
  const title =
    operation.kind === "cma_provisioning" ? "CMA setup" : "CMA task";
  return (
    <CmaCardFrame
      title={title}
      ariaLabel={`CMA submission: ${title}`}
      controlSubject="CMA submission"
      operationId={operation.id}
      collapsed={collapsed}
      onToggle={() => setCollapsed((value) => !value)}
      detailsId={detailsId}
      scrollBody={scrollBody}
      controls={
        <>
          <CmaExecutionControl operations={[operation]} />
          {cmaUrl ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  asChild
                  variant="ghost"
                  size="icon"
                  className="text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
                >
                  <a
                    href={cmaUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label="View in CMA"
                  >
                    <ExternalLink aria-hidden="true" />
                  </a>
                </Button>
              </TooltipTrigger>
              <TooltipContent>View in CMA</TooltipContent>
            </Tooltip>
          ) : null}
        </>
      }
    >
      <CmaOperationProgress operation={operation} />
      <div id={detailsId} hidden={collapsed} className="space-y-2">
        <CmaOperationDetails
          operation={operation}
          scrollBody={scrollBody}
          visible={!collapsed}
        />
      </div>
    </CmaCardFrame>
  );
}

const LONG_REQUEST_MS = 10 * 60_000;

/** Keep request status and notices when a live card replaces its receipt. */
export function CmaOperationProgress({
  operation,
  animate = true,
}: {
  readonly operation: CmaOperation;
  readonly animate?: boolean;
}) {
  const observed = presentation(operation);
  const pending = isCmaOperationPending(operation);
  const [now, setNow] = useState(Date.now);
  const running =
    pending &&
    operation.kind === "cma_response" &&
    observed.label === "Working";
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => {
      setNow(Date.now());
    }, 60_000);
    return () => {
      window.clearInterval(timer);
    };
  }, [running]);
  // Elapsed request time never changes the persisted remote outcome.
  const createdAt = Date.parse(operation.created_at);
  const takingLonger =
    running && Number.isFinite(createdAt) && now - createdAt >= LONG_REQUEST_MS;
  const { label, tone } = takingLonger
    ? { label: "Taking longer than usual", tone: "warning" }
    : observed;
  return (
    <>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span
          role="status"
          aria-live="polite"
          className="flex items-center gap-1.5 text-13 text-foreground"
        >
          {animate &&
          pending &&
          tone === "active" &&
          ["Submitting", "Starting", "Working"].includes(label) ? (
            <Loader2
              aria-hidden="true"
              className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none text-primary-text"
            />
          ) : label === "Queued" ? (
            <Clock3
              aria-hidden="true"
              className="h-3.5 w-3.5 text-text-tertiary"
            />
          ) : (
            <Circle
              aria-hidden="true"
              className={cn("h-2 w-2 fill-current", tones[tone])}
            />
          )}
          {label}
        </span>
        <span aria-hidden="true" className="text-text-tertiary">
          ·
        </span>
        <span className="text-13 text-muted-foreground">
          {pending ? "Tracking progress" : "Status recorded"}
        </span>
      </div>
      {takingLonger ? (
        <p className="text-13 text-warning">
          This request has been open for at least 10 minutes. CMA still reports
          it as running. View its progress, or ask NyxBot to cancel this task.
        </p>
      ) : null}
      {operation.reason === "timeout" ? (
        <p className="text-13 text-warning">
          Automatic tracking reached its time limit. CMA may still be running.
          Check its status before submitting it again.
        </p>
      ) : null}
      {tone === "warning" && !pending && operation.reason !== "timeout" ? (
        <p className="text-13 text-muted-foreground">
          CMA’s final outcome could not be confirmed. Check the operation before
          submitting it again.
        </p>
      ) : null}
      {operation.phase && phases[operation.phase] && pending ? (
        <p className="text-13 text-muted-foreground">
          {phases[operation.phase]}
        </p>
      ) : null}
      {tone === "success" && operation.reason?.startsWith("result_") ? (
        <p className="text-13 text-muted-foreground">
          CMA finished, but its final result is unavailable here.
        </p>
      ) : null}
      {operation.state === "cancelled" ? (
        <p className="text-13 text-muted-foreground">
          Tracking ended. CMA may still be running.
        </p>
      ) : null}
    </>
  );
}

export function CmaOperationDetails({
  operation,
  scrollBody,
  visible,
}: {
  readonly operation: CmaOperation;
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly visible: boolean;
}) {
  const lastChecked = operation.checked_at
    ? new Date(operation.checked_at)
    : null;
  return (
    <CmaCardDetails
      label="Submission details"
      scrollBody={scrollBody}
      visible={visible}
    >
      <div>
        <dt>Monitored by</dt>
        <dd className="mt-0.5 break-words text-muted-foreground">
          {operation.monitor_name}
        </dd>
      </div>
      {operation.agent_id ? (
        <div>
          <dt>Session ID</dt>
          <dd className="mt-0.5 break-all font-mono text-muted-foreground">
            {operation.agent_id}
          </dd>
        </div>
      ) : null}
      {operation.operation_id &&
      operation.operation_id !== operation.agent_id ? (
        <div>
          <dt>Operation ID</dt>
          <dd className="mt-0.5 break-all font-mono text-muted-foreground">
            {operation.operation_id}
          </dd>
        </div>
      ) : null}
      {operation.idempotency_key ? (
        <div>
          <dt>Idempotency key</dt>
          <dd className="mt-0.5 break-all font-mono text-muted-foreground">
            {operation.idempotency_key}
          </dd>
        </div>
      ) : null}
      {lastChecked && Number.isFinite(lastChecked.getTime()) ? (
        <div>
          <dt>Last checked</dt>
          <dd className="mt-0.5 text-muted-foreground">
            <time dateTime={operation.checked_at!}>
              {lastChecked.toLocaleString()}
            </time>
          </dd>
        </div>
      ) : null}
    </CmaCardDetails>
  );
}

function hasExecutionReference(operation: CmaOperation) {
  return Boolean(
    operation.agent_ref_id && /^[0-9a-f-]{36}$/i.test(operation.agent_ref_id),
  );
}

function orderedRequests(operations: readonly CmaOperation[]) {
  return [...operations].sort(
    (a, b) =>
      Number(isCmaOperationPending(b)) - Number(isCmaOperationPending(a)) ||
      Date.parse(b.created_at) - Date.parse(a.created_at),
  );
}

export function CmaExecutionControl({
  operations,
}: {
  readonly operations: readonly CmaOperation[];
}) {
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const operation = orderedRequests(operations).find(hasExecutionReference);
  if (!enabled || !operation) return null;
  return (
    <CmaAgentPanel
      key={`${operation.agent_ref_id}:${operation.response_id ?? ""}`}
      agentId={operation.agent_ref_id!}
      responseId={operation.response_id ?? undefined}
    />
  );
}

export function CmaSessionRequests({
  operations,
  scrollBody,
  visible,
}: {
  readonly operations: readonly CmaOperation[];
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly visible: boolean;
}) {
  const [limit, setLimit] = useState(3);
  const ordered = orderedRequests(operations);
  if (!ordered.length) return null;
  return (
    <section aria-label="CMA requests" className="space-y-2">
      <h4 className="type-group-label text-text-tertiary">Request status</h4>
      {ordered.slice(0, limit).map((operation) => (
        <section
          key={operation.id}
          aria-label={`CMA request: ${operation.monitor_name}, ${operation.created_at}`}
          data-cma-request={operation.id}
          className="space-y-1"
        >
          <p className="text-13 text-muted-foreground">
            {operation.monitor_name} ·{" "}
            <time dateTime={operation.created_at}>
              {Number.isFinite(Date.parse(operation.created_at))
                ? new Date(operation.created_at).toLocaleString()
                : "Time unavailable"}
            </time>
          </p>
          <CmaOperationProgress operation={operation} animate={false} />
          <CmaOperationDetails
            operation={operation}
            scrollBody={scrollBody}
            visible={visible}
          />
        </section>
      ))}
      {ordered.length > limit ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setLimit((value) => value + 3)}
        >
          Show older requests ({ordered.length - limit})
        </Button>
      ) : null}
    </section>
  );
}

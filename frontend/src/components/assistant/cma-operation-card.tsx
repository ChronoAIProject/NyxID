import { useEffect, useState } from "react";
import { safeCmaUrl } from "@/lib/assistant/cma-url";
import { Check, CircleAlert, Clock3, ExternalLink, Radio, Workflow } from "lucide-react";
import { STATUS_CARD_MAX_W } from "@/lib/assistant/banner";
import { CmaGlyph } from "@/components/service-icons/cma";
import { CmaAgentPanel } from "./cma-agent-panel";
import { Button } from "@/components/ui/button";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { cn } from "@/lib/utils";
import { isCmaOperationPending, type CmaOperation } from "@/schemas/cma-operation";

function presentation(operation: CmaOperation) {
  const { state, reason, status, kind } = operation;
  if (state === "cancelled") return { label: "Tracking stopped", tone: "muted" };
  if (state === "cancelling") return { label: "Stopping tracking", tone: "muted" };
  if (reason === "timeout") return { label: "Tracking timed out", tone: "warning" };
  if (kind === "cma_response" && reason === "operation_cancelled" && status === "incomplete") {
    return { label: "Cancelled", tone: "muted" };
  }
  if (reason === "submission_failed") return { label: "Submission failed", tone: "error" };
  if (reason === "submission_uncertain") {
    return { label: "Submission unconfirmed", tone: "warning" };
  }
  if (reason === "operation_failed" || ["failed", "incomplete", "cancelled", "stopped", "deleted"].includes(status ?? "")) {
    const label = status === "incomplete" ? "Incomplete"
      : status === "cancelled" || status === "stopped" ? "Stopped"
      : status === "deleted" ? "Deleted"
      : kind === "cma_response" ? "Failed" : "Needs attention";
    return { label, tone: ["cancelled", "stopped", "deleted"].includes(status ?? "") ? "muted" : "error" };
  }
  const succeeded = status === (kind === "cma_response" ? "completed" : kind === "cma_provisioning" ? "succeeded" : "ready");
  if (reason && reason !== "completed" && !(reason.startsWith("result_") && succeeded)) {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (succeeded) {
    return { label: kind === "cma_response" ? "Completed" : "Ready", tone: kind === "cma_agent" && isCmaOperationPending(operation) ? "neutral" : "success" };
  }
  if (reason === "completed" || ["completed", "succeeded", "ready"].includes(status ?? "")) {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (["waiting_approval", "connection_required", "awaiting_connection", "waiting_for_connection"].includes(status ?? "")) {
    return { label: status === "waiting_approval" ? "Needs approval" : "Needs connection", tone: "warning" };
  }
  if (status === "paused") return { label: "Paused", tone: "muted" };
  if (state === "submitting") return { label: "Submitting", tone: "active" };
  if (!status) return { label: "Awaiting status", tone: "active" };
  if (status === "queued") return { label: "Queued", tone: "active" };
  if (["starting", "provisioning"].includes(status)) return { label: "Starting", tone: "active" };
  if (["working", "running", "in_progress"].includes(status)) return { label: kind === "cma_provisioning" ? "Starting" : "Working", tone: "active" };
  return { label: "Status unavailable", tone: "warning" };
}
const tones: Record<string, string> = {
  active: "border-[#c6ef68]/25 bg-[#c6ef68]/10 text-[#d4f58b]",
  success: "border-emerald-300/20 bg-emerald-300/10 text-emerald-200",
  warning: "border-amber-300/25 bg-amber-300/10 text-amber-200",
  error: "border-rose-300/25 bg-rose-300/10 text-rose-200",
  muted: "border-white/15 bg-white/5 text-zinc-300",
  neutral: "border-border bg-muted text-muted-foreground",
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

const LONG_REQUEST_MS = 10 * 60_000;

export function CmaOperationCard({ operation }: { readonly operation: CmaOperation }) {
  const cmaEnabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const observed = presentation(operation);
  const pending = isCmaOperationPending(operation);
  const [now, setNow] = useState(Date.now);
  const running = pending && operation.kind === "cma_response" && observed.label === "Working";
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => { setNow(Date.now()); }, 60_000);
    return () => { window.clearInterval(timer); };
  }, [running]);
  // Elapsed request time is only a UX hint. A fresh poll is not proof of work
  // progress, and elapsed time never changes the persisted remote outcome.
  const createdAt = Date.parse(operation.created_at);
  const takingLonger = running && Number.isFinite(createdAt) && now - createdAt >= LONG_REQUEST_MS;
  const { label, tone } = takingLonger
    ? { label: "Taking longer than usual", tone: "warning" }
    : observed;
  const accepted = Boolean(operation.operation_id || (operation.kind === "cma_agent" && operation.status));
  const complete = tone === "success";
  const stages = [
    { label: "Submitted", done: accepted },
    { label: operation.kind === "cma_provisioning" ? "Provisioning" : "CMA working", done: complete },
    { label: operation.kind === "cma_response" ? "Result ready" : "Agent ready", done: complete },
  ];
  const lastChecked = operation.checked_at ? new Date(operation.checked_at) : null;
  const cmaUrl = safeCmaUrl(operation.cma_url);
  const executionAgentId = cmaEnabled && operation.agent_ref_id && /^[0-9a-f-]{36}$/i.test(operation.agent_ref_id)
    ? operation.agent_ref_id
    : undefined;
  return (
    <section
      aria-label="CMA delegation"
      data-cma-operation={operation.id}
      className={`${STATUS_CARD_MAX_W} relative my-3 overflow-hidden rounded-2xl border border-white/10 bg-[#111211] text-zinc-100 shadow-lg shadow-black/10`}
    >
      <div aria-hidden className="pointer-events-none absolute -right-16 -top-28 h-52 w-72 rounded-full bg-[#c6ef68]/[0.06] blur-3xl" />
      <div className="relative p-4 sm:p-5">
        <div className="flex flex-wrap items-center gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-full border border-white/5 bg-white/5 text-[#c6ef68]">
            <CmaGlyph className="size-6" />
          </span>
          <h3 className="min-w-0 flex-1 type-body-strong tracking-tight">Delegated to CMA</h3>
          <span role="status" aria-live="polite" className={cn("inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 type-secondary-emphasis ", tones[tone])}>
            <span aria-hidden className={cn("size-1.5 rounded-full bg-current", pending && tone === "active" && "motion-safe:animate-pulse")} />
            {label}
          </span>
        </div>
        <p className="mt-3 text-13 text-zinc-400">ChronoAI Managed Agent</p>
        <dl className="mt-1.5 space-y-1 break-all font-mono text-12 text-zinc-400">
          {operation.agent_id && <div><dt className="inline text-zinc-500">Agent </dt><dd className="inline select-all">{operation.agent_id}</dd></div>}
          {operation.operation_id && operation.operation_id !== operation.agent_id && <div><dt className="inline text-zinc-500">Operation </dt><dd className="inline select-all">{operation.operation_id}</dd></div>}
          {operation.idempotency_key && <div><dt className="inline text-zinc-500">Idempotency-Key </dt><dd className="inline select-all">{operation.idempotency_key}</dd></div>}
        </dl>
        <ol aria-label="CMA progress" className="mt-4 flex flex-wrap items-center gap-2 text-12">
          {stages.map((stage, index) => <li key={stage.label} className="flex items-center gap-2">
            {index > 0 && <span aria-hidden className="text-zinc-600">→</span>}
            <span className={cn("inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5", stage.done ? "border-white/10 bg-white/[0.06] text-zinc-200" : "border-white/5 text-zinc-500")}>
              {stage.done ? <Check aria-hidden className="size-3 text-[#c6ef68]" /> : <span aria-hidden className="size-1 rounded-full bg-current" />}
              {stage.label}
            </span>
          </li>)}
        </ol>
        {operation.phase && phases[operation.phase] && pending && <p className="mt-3 flex items-center gap-1.5 text-13 text-zinc-400"><Workflow aria-hidden className="size-3" />{phases[operation.phase]}</p>}
        {takingLonger && <p className="mt-3 flex items-start gap-1.5 text-13 text-amber-200/80"><CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />This request has been open for at least 10 minutes. CMA still reports it as running. View its progress, or ask NyxBot to cancel this task.</p>}
        {operation.reason === "timeout" && <p className="mt-3 flex items-start gap-1.5 text-13 text-amber-200/80"><CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />Automatic tracking reached its time limit. CMA may still be running. Check its status before submitting it again.</p>}
        {tone === "warning" && !pending && operation.reason !== "timeout" && <p className="mt-3 flex items-start gap-1.5 text-13 text-amber-200/80"><CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />CMA’s final outcome could not be confirmed. Check the operation before submitting it again.</p>}
        {tone === "success" && operation.reason?.startsWith("result_") && <p className="mt-3 text-13 text-zinc-400">CMA finished, but its final result is unavailable here.</p>}
        {operation.state === "cancelled" && <p className="mt-3 text-13 text-zinc-400">Tracking ended. CMA may still be running.</p>}
        {executionAgentId && <CmaAgentPanel
          key={`${executionAgentId}:${operation.response_id ?? ""}`}
          agentId={executionAgentId}
          responseId={operation.response_id ?? undefined}
        />}
        {cmaUrl && <Button asChild variant={executionAgentId ? "outline" : "primary"} className="mt-4 w-full">
          <a href={cmaUrl} target="_blank" rel="noopener noreferrer">
            View in CMA <ExternalLink aria-hidden className="size-3" />
          </a>
        </Button>}
        <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-white/[0.08] pt-3 text-12 text-zinc-400">
          <span className="inline-flex min-w-0 flex-1 items-center gap-2">
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-violet-400/15 text-violet-300"><Radio aria-hidden className="size-3" /></span>
            <span className="break-words"><span className="font-medium text-zinc-200">{operation.monitor_name}</span> · {pending ? "Tracking progress" : "Status recorded"}</span>
          </span>
          {lastChecked && Number.isFinite(lastChecked.getTime()) && <span className="inline-flex items-center gap-1"><Clock3 aria-hidden className="size-3" /><time dateTime={operation.checked_at!} title={lastChecked.toLocaleString()}>Checked {lastChecked.toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" })} · {lastChecked.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time></span>}
        </div>
      </div>
    </section>
  );
}

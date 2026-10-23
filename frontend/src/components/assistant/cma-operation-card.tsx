import { Check, CircleAlert, Clock3, Radio, Workflow } from "lucide-react";
import { CmaGlyph } from "@/components/service-icons/cma";
import { cn } from "@/lib/utils";
import { isCmaOperationPending, type CmaOperation } from "@/schemas/cma-operation";

function presentation(operation: CmaOperation) {
  const { state, reason, status, kind } = operation;
  if (state === "cancelled") return { label: "Tracking stopped", tone: "muted" };
  if (state === "cancelling") return { label: "Stopping tracking", tone: "muted" };
  if (reason === "timeout") return { label: "Tracking timed out", tone: "warning" };
  if (["submission_uncertain", "submission_failed"].includes(reason ?? "")) {
    return { label: "Submission unconfirmed", tone: "warning" };
  }
  if (reason?.startsWith("result_") && ["completed", "succeeded", "ready"].includes(status ?? "")) {
    return { label: kind === "cma_response" ? "Completed" : "Agent ready", tone: "success" };
  }
  if (reason === "operation_failed" || ["failed", "incomplete", "cancelled", "stopped", "deleted"].includes(status ?? "")) {
    return { label: status === "incomplete" ? "Incomplete" : status === "cancelled" || status === "stopped" ? "Stopped" : "Failed", tone: "error" };
  }
  if (reason && reason !== "completed") {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (status === "completed" || reason === "completed") {
    return { label: kind === "cma_response" ? "Completed" : "Agent ready", tone: "success" };
  }
  if (status === "ready" && kind === "cma_agent" && isCmaOperationPending(operation)) {
    return { label: "Agent ready", tone: "active" };
  }
  if (status === "succeeded" || status === "ready") return { label: "Agent ready", tone: "success" };
  if (["waiting_approval", "connection_required", "awaiting_connection", "waiting_for_connection"].includes(status ?? "")) {
    return { label: status === "waiting_approval" ? "Needs approval" : "Needs connection", tone: "warning" };
  }
  if (status === "paused") return { label: "Paused", tone: "warning" };
  if (state === "submitting") return { label: "Submitting", tone: "active" };
  if (!status) return { label: "Awaiting status", tone: "active" };
  if (status === "queued") return { label: "Queued", tone: "active" };
  return { label: kind === "cma_provisioning" ? "Provisioning" : "Running", tone: "active" };
}

const tones: Record<string, string> = {
  active: "border-[#c6ef68]/25 bg-[#c6ef68]/10 text-[#d4f58b]",
  success: "border-emerald-300/20 bg-emerald-300/10 text-emerald-200",
  warning: "border-amber-300/25 bg-amber-300/10 text-amber-200",
  error: "border-rose-300/25 bg-rose-300/10 text-rose-200",
  muted: "border-white/15 bg-white/5 text-zinc-300",
};

const phases: Record<string, string> = {
  sandbox_creation: "Creating sandbox",
  sandbox_startup: "Starting sandbox",
  workspace_creation: "Preparing workspace",
  agent_creation: "Creating agent",
  agent_startup: "Starting agent",
};

export function CmaOperationCard({ operation }: { readonly operation: CmaOperation }) {
  const { label, tone } = presentation(operation);
  const pending = isCmaOperationPending(operation);
  const accepted = Boolean(operation.operation_id || (operation.kind === "cma_agent" && operation.status));
  const complete = tone === "success";
  const stages = [
    { label: "Submitted", done: accepted },
    { label: operation.kind === "cma_provisioning" ? "Provisioning" : "CMA working", done: complete },
    { label: operation.kind === "cma_response" ? "Result ready" : "Agent ready", done: complete },
  ];
  const lastChecked = operation.checked_at ? new Date(operation.checked_at) : null;
  return (
    <section
      aria-label="CMA delegation"
      data-cma-operation={operation.id}
      className="relative my-3 overflow-hidden rounded-2xl border border-white/10 bg-[#111211] text-zinc-100 shadow-lg shadow-black/10"
    >
      <div aria-hidden className="pointer-events-none absolute -right-16 -top-28 h-52 w-72 rounded-full bg-[#c6ef68]/[0.06] blur-3xl" />
      <div className="relative p-4 sm:p-5">
        <div className="flex flex-wrap items-center gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-full border border-white/5 bg-white/5 text-[#c6ef68]">
            <CmaGlyph className="size-6" />
          </span>
          <h3 className="min-w-0 flex-1 text-sm font-semibold tracking-tight">Delegated to CMA</h3>
          <span role="status" aria-live="polite" className={cn("inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-11 font-medium", tones[tone])}>
            <span aria-hidden className={cn("size-1.5 rounded-full bg-current", pending && tone === "active" && "motion-safe:animate-pulse")} />
            {label}
          </span>
        </div>
        <p className="mt-3 text-11 text-zinc-400">ChronoAI Managed Agent</p>
        <dl className="mt-1.5 space-y-1 break-all font-mono text-10 leading-relaxed text-zinc-400">
          {operation.agent_id && <div><dt className="inline text-zinc-500">Agent </dt><dd className="inline select-all">{operation.agent_id}</dd></div>}
          {operation.operation_id && operation.operation_id !== operation.agent_id && <div><dt className="inline text-zinc-500">Operation </dt><dd className="inline select-all">{operation.operation_id}</dd></div>}
          {operation.idempotency_key && <div><dt className="inline text-zinc-500">Idempotency-Key </dt><dd className="inline select-all">{operation.idempotency_key}</dd></div>}
        </dl>
        <ol aria-label="CMA progress" className="mt-4 flex flex-wrap items-center gap-2 text-10">
          {stages.map((stage, index) => <li key={stage.label} className="flex items-center gap-2">
            {index > 0 && <span aria-hidden className="text-zinc-600">→</span>}
            <span className={cn("inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5", stage.done ? "border-white/10 bg-white/[0.06] text-zinc-200" : "border-white/5 text-zinc-500")}>
              {stage.done ? <Check aria-hidden className="size-3 text-[#c6ef68]" /> : <span aria-hidden className="size-1 rounded-full bg-current" />}
              {stage.label}
            </span>
          </li>)}
        </ol>
        {operation.phase && phases[operation.phase] && pending && <p className="mt-3 flex items-center gap-1.5 text-11 text-zinc-400"><Workflow aria-hidden className="size-3" />{phases[operation.phase]}</p>}
        {tone === "warning" && !pending && <p className="mt-3 flex items-start gap-1.5 text-11 text-amber-200/80"><CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />CMA’s final outcome could not be confirmed. Check the operation before submitting it again.</p>}
        {tone === "success" && operation.reason?.startsWith("result_") && <p className="mt-3 text-11 text-zinc-400">CMA finished, but its final result is unavailable here.</p>}
        {operation.state === "cancelled" && <p className="mt-3 text-11 text-zinc-400">Tracking ended. CMA may still be running.</p>}
        <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-white/[0.08] pt-3 text-10 text-zinc-400">
          <span className="inline-flex min-w-0 flex-1 items-center gap-2">
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-violet-400/15 text-violet-300"><Radio aria-hidden className="size-3" /></span>
            <span className="break-words"><span className="font-medium text-zinc-200">{operation.monitor_name}</span> · {pending ? "Tracking progress" : "Status recorded"}</span>
          </span>
          {lastChecked && Number.isFinite(lastChecked.getTime()) && <span className="inline-flex items-center gap-1"><Clock3 aria-hidden className="size-3" /><time dateTime={operation.checked_at!} title={lastChecked.toLocaleString()}>Checked {lastChecked.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time></span>}
        </div>
      </div>
    </section>
  );
}

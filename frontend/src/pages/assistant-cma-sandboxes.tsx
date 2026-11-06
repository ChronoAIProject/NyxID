import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  ArrowLeft,
  ArrowRight,
  Box,
  Clock3,
  Cpu,
  MemoryStick,
  Pause,
  Pencil,
  Play,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAppForm } from "@/components/ui/form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  useCmaSandboxes,
  useCmaSandboxCommand,
  useRenameCmaSandbox,
  useRetireCmaSandbox,
} from "@/hooks/use-nyxbot-cma-sandboxes";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  cmaSandboxLabelSchema,
  type CmaSandbox,
} from "@/schemas/nyxbot-cma-sandbox";
import { cn } from "@/lib/utils";

type Action = "rename" | "pause" | "revive" | "retire";
const statusLabels: Record<string, string> = {
  ready: "Running",
  provisioning: "Preparing",
  reserved: "Preparing",
  creating: "Preparing",
  unknown: "Not confirmed",
  paused: "Paused",
  pausing: "Pausing",
  reviving: "Resuming",
  resuming: "Resuming",
  retiring: "Retiring",
  retired: "Retired",
  failed: "Needs attention",
};

function stateOf(sandbox: CmaSandbox) {
  if (sandbox.admission_state === "retired") return "retired";
  if (
    sandbox.admission_state === "associated" &&
    ["pausing", "reviving", "resuming"].includes(sandbox.resource_state ?? "")
  )
    return sandbox.resource_state!;
  return ["reserved", "creating", "retiring", "unknown"].includes(
    sandbox.admission_state,
  )
    ? sandbox.admission_state
    : sandbox.observation_state;
}
function dateLabel(value: string | null) {
  if (!value || !Number.isFinite(new Date(value).getTime()))
    return "Unavailable";
  return new Date(value).toLocaleString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function AssistantCmaSandboxesPage() {
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const query = useCmaSandboxes();
  if (!enabled)
    return (
      <p role="status" className="text-13 text-text-tertiary">
        Sandboxes are not enabled for this account.
      </p>
    );
  const rows =
    query.data?.sandboxes.filter(
      (row) => !row.retired_at && row.admission_state !== "retired",
    ) ?? [];
  const confirmedEmpty =
    query.data?.inventory_complete &&
    query.data.occupied === 0 &&
    !query.data.conflict &&
    rows.length === 0;
  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="mb-2 type-secondary text-text-tertiary">Workspace</p>
          <h1 className="type-compact-page-title sm:type-page-title tracking-tight">Sandboxes</h1>
          <p className="mt-2 text-13 text-text-tertiary">
            Only Sandboxes created through NyxID appear here and use this
            allocation.
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() => void query.refetch()}
            isLoading={query.isFetching}
          >
            <RefreshCw />
            Refresh
          </Button>
        </div>
      </header>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-hairline px-4 py-3 text-12">
        <span className="flex items-center gap-2.5">
          <ShieldCheck className="size-4 text-text-tertiary" />
          Personal account{" "}
          <span className="text-text-tertiary">/ NyxID Sandbox allocation</span>
        </span>
        <span className="font-medium tabular-nums">
          {query.data?.occupied ?? "—"}{" "}
          <span className="text-text-tertiary">
            / {query.data?.limit ?? "—"} used
          </span>
        </span>
      </div>
      {query.isPending && <p role="status">Checking your environment…</p>}
      {query.error && (
        <p role="alert" className="text-13 text-destructive">
          {query.error.message}
        </p>
      )}
      {query.data && !query.data.inventory_complete && (
        <Notice>
          Live resource status could not be fully confirmed. Existing
          allocations remain reserved until CMA confirms the outcome.
        </Notice>
      )}
      {query.data?.conflict && (
        <Notice>
          <strong className="font-medium">
            Your account has multiple Sandboxes created through NyxID.
          </strong>
          <p className="mt-1">
            Review each Sandbox and retire those you no longer need. New
            Sandboxes through NyxID are blocked until the conflict is resolved.
          </p>
        </Notice>
      )}
      {confirmedEmpty && (
        <section className="rounded-xl border border-hairline bg-card px-6 py-14 text-center shadow-sm">
          <Box
            className="mx-auto mb-5 size-10 text-text-tertiary"
            strokeWidth={1.25}
          />
          <h2 className="type-section-heading">
            {query.data?.sandboxes.some((row) => row.retired_at)
              ? "Environment retired"
              : "Room for your next task"}
          </h2>
          <p className="mx-auto mt-3 max-w-sm type-secondary text-text-tertiary">
            NyxBot creates an environment when a task needs it. Simply opening
            this page doesn’t start one.
          </p>
          <Button asChild variant="primary" className="mt-6">
            <Link to="/assistant">
              Ask NyxBot
              <ArrowRight />
            </Link>
          </Button>
        </section>
      )}
      {rows.map((sandbox) => (
        <CmaSandboxCard key={sandbox.id} sandbox={sandbox} />
      ))}
    </div>
  );
}

function Notice({ children }: { readonly children: React.ReactNode }) {
  return (
    <div
      role="status"
      className="rounded-lg border border-warning/30 bg-warning/5 p-4 type-secondary text-muted-foreground"
    >
      {children}
    </div>
  );
}

export function CmaSandboxCard({
  sandbox,
  detail = false,
}: {
  readonly sandbox: CmaSandbox;
  readonly detail?: boolean;
}) {
  const [action, setAction] = useState<Action>();
  const status = stateOf(sandbox);
  const pending = [
    "reserved",
    "creating",
    "provisioning",
    "retiring",
    "pausing",
    "reviving",
    "resuming",
  ].includes(status);
  const unknown = status === "unknown";
  const retired = status === "retired" || Boolean(sandbox.retired_at);
  const actionable =
    Boolean(sandbox.cma_sandbox_id) && !pending && !unknown && !retired;
  const capacitySource = sandbox.capacity?.source;
  const capacityConfirmed =
    Boolean(sandbox.cma_sandbox_id) &&
    (capacitySource === "profile_revision" ||
      capacitySource === "runtime_allocation");
  const capacity = (
    value: number | null | undefined,
    divisor: number,
    unit: string,
  ) =>
    capacityConfirmed && value != null && value > 0
      ? `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value / divisor)} ${unit}`
      : "Not confirmed";
  return (
    <>
      {detail && (
        <Link
          to="/assistant/sandboxes"
          className="mb-4 inline-flex items-center gap-1.5 text-12 text-text-tertiary"
        >
          <ArrowLeft className="size-3" />
          Sandboxes
        </Link>
      )}
      <section
        aria-label={`Environment ${sandbox.label}`}
        className="overflow-hidden rounded-xl border border-hairline bg-card shadow-sm"
      >
        <div className="flex flex-wrap items-start justify-between gap-3 p-5 sm:p-6">
          <div className="flex min-w-0 items-center gap-3.5">
            <span className="flex size-12 shrink-0 items-center justify-center rounded-xl border border-hairline bg-overlay">
              <Box className="size-5 text-text-tertiary" />
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2.5">
                <h2 className="break-words type-card-title">
                  {sandbox.label}
                </h2>
                <span
                  className={cn(
                    "rounded-full border border-hairline px-2 py-0.5 type-badge",
                    status === "ready"
                      ? "text-success"
                      : unknown || pending
                        ? "text-warning"
                        : "text-text-tertiary",
                  )}
                >
                  {statusLabels[status] ?? "Not confirmed"}
                </span>
              </div>
              <p className="mt-1 break-all font-mono type-secondary text-text-tertiary">
                {sandbox.cma_sandbox_id ?? "Environment identity pending"}
              </p>
            </div>
          </div>
          <Button
            size="icon"
            variant="ghost"
            aria-label="Rename environment"
            onClick={() => setAction("rename")}
            disabled={retired}
          >
            <Pencil className="size-4" />
          </Button>
        </div>
        {(pending || unknown) && (
          <div className="mx-5 mb-5 sm:mx-6">
            <Notice>
              {unknown
                ? "We haven’t confirmed the resource state. This allocation remains reserved. Check the existing request before starting a new environment."
                : status === "retiring"
                  ? "Retirement requested. Waiting for confirmation. Your allocation stays in use until retirement is confirmed."
                  : "Preparing your environment. This allocation stays in use while the request is being confirmed."}
            </Notice>
          </div>
        )}
        <div className="border-y border-hairline bg-overlay px-5 py-5 sm:px-6">
          <h3 className="mb-4 type-group-label text-text-tertiary">
            Resources
          </h3>
          <dl className="grid grid-cols-2 gap-5">
            <div>
              <dt className="flex items-center gap-1.5 type-secondary text-text-tertiary">
                <Cpu className="size-3" />
                CPU
              </dt>
              <dd className="mt-2 type-secondary-emphasis">
                {capacity(sandbox.capacity?.cpu_millis, 1000, "vCPU")}
              </dd>
            </div>
            <div>
              <dt className="flex items-center gap-1.5 type-secondary text-text-tertiary">
                <MemoryStick className="size-3" />
                Memory
              </dt>
              <dd className="mt-2 type-secondary-emphasis">
                {capacity(sandbox.capacity?.memory_mib, 1024, "GiB")}
              </dd>
            </div>
          </dl>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-4 sm:px-6">
          <div className="space-y-1 type-meta text-text-tertiary">
            <p className="flex items-center gap-1.5">
              <Clock3 className="size-3" />
              CMA · Last active {dateLabel(sandbox.last_activity_at)}
            </p>
            <p>Last observed {dateLabel(sandbox.last_observed)}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={!actionable || !["ready", "paused"].includes(status)}
              onClick={() =>
                setAction(status === "paused" ? "revive" : "pause")
              }
            >
              {status === "paused" ? <Play /> : <Pause />}
              {status === "paused" ? "Resume" : "Pause"}
            </Button>
            {!detail && (
              <Button asChild>
                <Link
                  to="/assistant/sandboxes/$sandboxId"
                  params={{ sandboxId: sandbox.id }}
                >
                  View details
                  <ArrowRight />
                </Link>
              </Button>
            )}
            <Button
              variant="destructive"
              disabled={!actionable}
              onClick={() => setAction("retire")}
            >
              <Trash2 />
              Retire environment
            </Button>
          </div>
        </div>
      </section>
      {detail && (
        <p className="mt-4 type-secondary text-text-tertiary">
          Retirement permanently removes this environment and its Agents,
          workspaces and files. CMA determines final billing and retained
          charges.
        </p>
      )}
      {action && (
        <SandboxActionDialog
          key={`${sandbox.id}:${action}`}
          sandbox={sandbox}
          action={action}
          onClose={() => setAction(undefined)}
        />
      )}
    </>
  );
}

function SandboxActionDialog({
  sandbox,
  action,
  onClose,
}: {
  readonly sandbox: CmaSandbox;
  readonly action: Action;
  readonly onClose: () => void;
}) {
  const rename = useRenameCmaSandbox();
  const command = useCmaSandboxCommand();
  const retire = useRetireCmaSandbox();
  const form = useAppForm({
    resolver: zodResolver(cmaSandboxLabelSchema),
    defaultValues: { label: sandbox.label },
  });
  const [confirmation, setConfirmation] = useState("");
  const [key] = useState(() => crypto.randomUUID());
  const [error, setError] = useState<string>();
  const [submitting, setSubmitting] = useState(false);
  const title =
    action === "rename"
      ? "Rename environment"
      : action === "pause"
        ? "Pause environment"
        : action === "revive"
          ? "Resume environment"
          : "Retire environment";
  async function submit() {
    setSubmitting(true);
    setError(undefined);
    try {
      if (action === "rename") {
        const parsed = cmaSandboxLabelSchema.parse(form.getValues());
        await rename.mutateAsync({
          id: sandbox.id,
          generation: sandbox.generation,
          label: parsed.label,
        });
        toast.success("Environment name updated");
      } else {
        const identity = {
          id: sandbox.id,
          generation: sandbox.generation,
          idempotencyKey: key,
        };
        const result =
          action === "retire"
            ? await retire.mutateAsync(identity)
            : await command.mutateAsync({ ...identity, command: action });
        toast.success(
          result.pending
            ? "Request accepted. Waiting for CMA confirmation."
            : "CMA resource status updated.",
        );
      }
      onClose();
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "The request could not be confirmed. Refresh before retrying.",
      );
    } finally {
      setSubmitting(false);
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {action === "rename"
              ? "This is a local display name. The CMA resource ID stays the same."
              : action === "retire"
                ? "Permanently remove this environment, all Agents, workspaces and files. Submitting retirement does not mean cleanup or billing has completed."
                : action === "pause"
                  ? "All Agents in this Sandbox may become unavailable. Active work and shared desktop sessions may be interrupted."
                  : "CMA will resume the environment. Task continuation depends on each Agent’s state."}
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={
            action === "rename"
              ? form.handleSubmit(submit)
              : (event) => {
                  event.preventDefault();
                  if (action !== "retire" || confirmation === sandbox.label)
                    void submit();
                }
          }
          className="space-y-4"
        >
          {action === "rename" && (
            <div className="space-y-2">
              <Label htmlFor="cma-label">Environment name</Label>
              <Input
                id="cma-label"
                maxLength={100}
                {...form.register("label")}
              />
              {form.formState.errors.label && (
                <p role="alert" className="text-12 text-destructive">
                  {form.formState.errors.label.message}
                </p>
              )}
            </div>
          )}
          {action === "retire" && (
            <div className="space-y-2">
              <Label htmlFor="cma-retire-confirm">
                Type {sandbox.label} to confirm
              </Label>
              <Input
                id="cma-retire-confirm"
                autoComplete="off"
                value={confirmation}
                onChange={(event) => setConfirmation(event.target.value)}
              />
            </div>
          )}
          {error && (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant={action === "retire" ? "destructive" : "primary"}
              isLoading={submitting}
              disabled={
                submitting ||
                (action === "retire" && confirmation !== sandbox.label) ||
                (action === "rename" && !form.formState.isDirty)
              }
            >
              {action === "rename" ? "Save name" : title}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

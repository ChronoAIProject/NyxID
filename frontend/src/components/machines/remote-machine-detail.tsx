import {
  assertRemoteMachineAdmission,
  useRemoteMachineAdmission,
} from "@/hooks/use-remote-machine-admission";
import { RemoteMachineServiceUsage } from "@/components/machines/remote-machine-service-usage";
import { useEffect, useState } from "react";
import { RemoteMachineQuota } from "./remote-machine-preflight";
import { RemoteMachinePlatformReadiness } from "./remote-machine-platform-readiness";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  AlertTriangle,
  Copy,
  Pause,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import {
  useRemoteMachine,
  useRemoteMachineAction,
  useRemoteMachineEvents,
  useRemoteMachineQuote,
  useRemoteMachineOperation,
  useRemoteMachineRuntimeBilling,
  useRemoteMachineEntitlement,
} from "@/hooks/use-machines";
import { useNode } from "@/hooks/use-nodes";
import {
  RemoteMachineOperationCard,
  type MachineAccountingView,
} from "./remote-machine-lifecycle";
export { RemoteMachineOperationCard } from "./remote-machine-lifecycle";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useAuthStore } from "@/stores/auth-store";
import { remoteMachineActionRequestSchema } from "@/schemas/machines";
import type { RemoteMachine, RemoteMachinePolicy } from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { ApiError } from "@/lib/api-client";
import {
  clearRemoteMachineIntent,
  getRemoteMachineIntent,
  readRemoteMachineIntent,
} from "@/lib/remote-machine-intent";
import { copyToClipboard } from "@/lib/utils";
import {
  buildClaudeCodeConfig,
  buildCodexConfig,
} from "@/pages/settings.helpers";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { MachineGrantList } from "@/components/shared/machine-grant-list";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { RemoteMachineAgentAccess } from "./remote-machine-agent-access";
import { RemoteMachineJobs } from "./remote-machine-jobs";
import { RemoteMachineBilling } from "./remote-machine-billing";
import { RemoteMachineConfirmations } from "./remote-machine-confirmations";
import { RemoteMachinePaidStart } from "./remote-machine-paid-start";
import { RemoteMachineUsage } from "./remote-machine-usage";

const actionReviewSchema = z.object({
  acknowledged: z
    .boolean()
    .refine(Boolean, "Review and acknowledge this action before continuing."),
});
type OwnerAction = "pause" | "resume" | "delete";
const capabilityLabels = {
  shell: "Commands",
  files: "Files",
  browser: "Browser",
  computer: "Computer",
} as const;

function readable(value: string) {
  return value.replaceAll("_", " ");
}
function time(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Not reported";
}
function statusVariant(
  value: string,
): "success" | "warning" | "destructive" | "secondary" {
  if (["ready", "active", "free", "succeeded", "complete"].includes(value))
    return "success";
  if (
    ["failed", "revoked", "suspended", "unavailable", "degraded"].includes(
      value,
    )
  )
    return "destructive";
  if (
    [
      "provisioning",
      "resuming",
      "stopping",
      "deleting",
      "reconciling",
      "pending",
      "accepted",
      "applying",
    ].includes(value)
  )
    return "warning";
  return "secondary";
}

type RemoteMachineDetailProps = {
  readonly machineId: string;
  readonly initialMachine?: RemoteMachine;
  readonly node?: NodeInfo;
  readonly policy?: RemoteMachinePolicy;
  readonly onClose?: () => void;
  readonly operationId?: string;
  readonly confirmationId?: string;
  readonly accessPanel?: React.ReactNode;
};

export function RemoteMachineDetail(props: RemoteMachineDetailProps) {
  const owner = useAuthStore((state) => state.user?.id);
  return (
    <RemoteMachineDetailView key={`${owner}:${props.machineId}`} {...props} />
  );
}

function RemoteMachineDetailView({
  machineId,
  initialMachine,
  node: suppliedNode,
  policy: suppliedPolicy,
  onClose,
  operationId: linkedOperationId,
  confirmationId,
  accessPanel,
}: RemoteMachineDetailProps) {
  const userId = useAuthStore((state) => state.user?.id);
  const remoteAdmission = useRemoteMachineAdmission();
  const query = useRemoteMachine(machineId);
  const candidate = query.data ?? initialMachine;
  const machine =
    !query.error && candidate?.node_id === machineId ? candidate : undefined;
  const node =
    suppliedNode?.id === machineId &&
    suppliedNode.owner.kind === "user" &&
    suppliedNode.owner.id === userId
      ? suppliedNode
      : undefined;
  const entitlement = useRemoteMachineEntitlement(
    Boolean(machine && !suppliedPolicy),
    machine?.profile.profile_version,
  );
  const policy =
    suppliedPolicy ??
    (!entitlement.error ? entitlement.data?.policy : undefined);
  const operationQuery = useRemoteMachineOperation(
    machine?.active_operation_id ?? undefined,
  );
  const activeOperation =
    !operationQuery.error &&
    operationQuery.data?.id === machine?.active_operation_id &&
    operationQuery.data?.machine_id === machineId
      ? operationQuery.data
      : undefined;
  const [selectedOperationId, setSelectedOperationId] = useState<string | null>(
    linkedOperationId ?? null,
  );
  const selectedQuery = useRemoteMachineOperation(
    selectedOperationId && selectedOperationId !== machine?.active_operation_id
      ? selectedOperationId
      : undefined,
  );
  const selectedOperation =
    selectedOperationId === activeOperation?.id
      ? activeOperation
      : selectedQuery.data?.machine_id === machineId &&
          selectedQuery.data.id === selectedOperationId
        ? selectedQuery.data
        : undefined;
  const [tab, setTab] = useState(confirmationId ? "confirmations" : "overview");
  const [reviewAction, setReviewAction] = useState<OwnerAction | null>(null);
  const [paidResumeOpen, setPaidResumeOpen] = useState(false);
  const [acceptedAction, setAcceptedAction] = useState<OwnerAction | null>(
    null,
  );
  if (!remoteAdmission && (reviewAction === "resume" || paidResumeOpen)) {
    setReviewAction(null);
    setPaidResumeOpen(false);
  }
  const freeSlot = machine?.free_slot ?? machine?.billing_state === "free";
  const billing = useRemoteMachineRuntimeBilling(
    machine && !freeSlot ? machine.node_id : undefined,
    machine?.runtime_run_id ?? undefined,
  );
  const billingData =
    !billing.error &&
    billing.data?.node_id === machineId &&
    (!machine?.runtime_run_id || billing.data.run_id === machine.runtime_run_id)
      ? billing.data
      : undefined;
  const accounting: MachineAccountingView = freeSlot
    ? {
        label: "Free runtime · no paid contract",
        detail: "Downstream services are charged independently.",
        closed: true,
      }
    : billingData
      ? {
          label:
            billingData.state === "closed"
              ? "Runtime contract closed"
              : `Runtime contract ${readable(billingData.state)}`,
          detail:
            billingData.state === "closed"
              ? "The runtime billing read confirms accounting closure."
              : "Accounting closure has not been confirmed.",
          closed: billingData.state === "closed",
        }
      : {
          label: "Accounting finality unknown",
          detail: billing.isPending
            ? "Reading the runtime billing contract…"
            : "The runtime contract could not be confirmed. Open Billing to review or retry.",
          closed: false,
        };
  const notifications = useRemoteMachineEvents(machine?.node_id);
  const notice = !notifications.error
    ? notifications.data?.events.find(
        (event) =>
          event.notification &&
          (!activeOperation || event.operation_id === activeOperation.id) &&
          !(
            event.kind === "accepted" &&
            activeOperation &&
            ["succeeded", "failed"].includes(activeOperation.state)
          ),
      )
    : undefined;

  async function refreshProgress() {
    const results = await Promise.allSettled([
      query.refetch(),
      operationQuery.refetch(),
    ]);
    return results.every(
      (result) => result.status === "fulfilled" && !result.value?.error,
    );
  }

  if (query.isPending && !machine)
    return (
      <div
        role="status"
        aria-label="Loading remote machine"
        className="space-y-4 p-5"
      >
        <Skeleton className="h-12 w-48" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  if (!machine)
    return (
      <div className="p-5">
        <ErrorBanner
          message={query.error?.message ?? "Remote machine not found."}
          onRetry={() => void query.refetch()}
        />
        {onClose && (
          <Button variant="ghost" onClick={onClose}>
            Back to machines
          </Button>
        )}
      </div>
    );

  const closed = machine.resource_state === "deleted";
  const canResume = machine.resource_state === "paused";
  const canPause = ["ready", "degraded"].includes(machine.resource_state);
  const desktopNode =
    node && (node.machine?.browser || node.machine?.computer)
      ? node
      : undefined;
  const desktopUnavailable = desktopNode
    ? !desktopNode.is_connected
      ? "Desktop unavailable while this machine is disconnected."
      : !canPause || machine.authorization_state !== "active"
        ? "Desktop unavailable until this machine has an active runtime."
        : null
    : null;
  const acceptancePending =
    acceptedAction &&
    activeOperation?.state !== "failed" &&
    !(
      (acceptedAction === "delete" && closed) ||
      (acceptedAction === "pause" && machine.resource_state === "paused") ||
      (acceptedAction === "resume" && machine.resource_state === "ready")
    );
  const title = closed
    ? machine.cleanup_state === "pending"
      ? "Background supplemental cleanup"
      : "Machine deleted"
    : machine.resource_state === "deleting"
      ? machine.cleanup_state === "complete"
        ? "Platform object cleanup confirmed"
        : "Deletion remains pending"
      : machine.resource_state === "paused"
        ? !remoteAdmission
          ? "Resume currently unavailable"
          : "Machine paused"
        : ["reconciling", "failed", "degraded"].includes(machine.resource_state)
          ? "Resource state needs attention"
          : ["provisioning", "resuming", "stopping"].includes(
                machine.resource_state,
              )
            ? `${readable(machine.resource_state)} · awaiting confirmation`
            : node && !node.is_connected
              ? "Machine connection unavailable"
              : null;
  const attention = closed
    ? machine.cleanup_state === "pending"
      ? "Later scans are checking supplemental cleanup. The machine stays deleted and the original deletion remains successful; execution stays closed and no new deletion retry is available."
      : "The platform confirmed Sandbox and related object cleanup, terminal creation responsibilities and accounting before final deletion. Physical disk release or erasure is not verified."
    : machine.resource_state === "deleting"
      ? machine.cleanup_state === "complete"
        ? accounting.closed
          ? "Platform objects are clear. Final deletion still requires the platform’s remaining finality checks."
          : billingData
            ? "Awaiting accounting closure. Platform object cleanup does not establish billing finality."
            : "Runtime accounting is independently checked below; its finality is currently unknown."
        : "Execution is closed. Platform object cleanup, creation responsibilities and runtime accounting must be confirmed independently."
      : machine.resource_state === "paused"
        ? !remoteAdmission
          ? "Remote Machines are not enabled for this account. Resume is unavailable. Existing records and deletion remain available."
          : "The workspace is retained. Resume starts a new runtime after review and does not restore old processes or jobs."
        : ["reconciling", "failed", "degraded"].includes(machine.resource_state)
          ? "The platform is checking resource and runtime evidence. A disconnected node does not prove the runtime has stopped."
          : node && !node.is_connected
            ? "Connection loss does not prove processes stopped or that charging ended. Resource and accounting observations remain separate."
            : "An accepted request is awaiting a confirmed resource result.";

  return (
    <div className="min-h-0 min-w-0 space-y-6 p-4 sm:p-5 [&_h2]:break-words [&>div:first-child>div:last-child]:max-w-full max-sm:[&_div.flex.text-12]:flex-col max-sm:[&_div.flex.text-12]:items-start max-sm:[&_div.flex.text-12]:gap-1 max-sm:[&_div.flex.text-12>div]:max-w-full max-sm:[&_.text-right]:text-left">
      <PageHeader
        title={machine.name ?? node?.name ?? "Remote Nyx Machine"}
        description={`${machine.profile.cpu_millis / 1000} vCPU · ${machine.profile.memory_mib / 1024} GiB RAM · ${machine.profile.workspace_gib} GiB workspace`}
        actions={
          <div className="flex flex-wrap gap-2">
            {canPause && (
              <Button onClick={() => setReviewAction("pause")}>
                <Pause className="size-3" />
                Pause machine
              </Button>
            )}
            {canResume && (
              <Button
                onClick={() =>
                  freeSlot ? setReviewAction("resume") : setPaidResumeOpen(true)
                }
                disabled={!remoteAdmission}
              >
                <Play className="size-3" />
                {freeSlot ? "Resume machine" : "Review quote to resume"}
              </Button>
            )}
            <Button
              size="icon"
              aria-label="Refresh machine"
              onClick={() => void refreshProgress()}
              disabled={query.isFetching}
            >
              <RefreshCw className="size-3" />
            </Button>
          </div>
        }
      />
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={statusVariant(machine.resource_state)}>
          {readable(machine.resource_state)}
        </Badge>
        <div
          role="group"
          aria-label="Enabled capability badges"
          className="flex flex-wrap gap-2"
        >
          {machine.capabilities?.length ? (
            machine.capabilities.map((capability) => (
              <Badge key={capability} variant="secondary">
                {capabilityLabels[capability]}
              </Badge>
            ))
          ) : (
            <Badge variant="secondary">Capabilities not reported</Badge>
          )}
        </div>
        <Badge variant="secondary">Shared environment</Badge>
        {onClose && (
          <Button variant="ghost" onClick={onClose}>
            Back to machines
          </Button>
        )}
      </div>
      {(title ||
        notice ||
        activeOperation ||
        operationQuery.error ||
        acceptancePending) && (
        <section
          aria-label="Current attention"
          className="space-y-4 rounded-xl border border-border bg-card p-4"
        >
          {title && (
            <div className="space-y-1">
              <h3 className="text-13 font-semibold">{title}</h3>
              <p className="text-12 text-muted-foreground">{attention}</p>
            </div>
          )}
          {notice && (
            <div className="space-y-1 text-12">
              <p>{notice.message}</p>
              <a
                className="break-all text-primary-text underline"
                href={`/assistant/machines?machine=${encodeURIComponent(machineId)}&operation=${encodeURIComponent(notice.operation_id)}`}
              >
                View operation {notice.operation_id}
              </a>
            </div>
          )}
          {acceptancePending && acceptedAction && (
            <p role="status" className="text-12">
              {readable(acceptedAction)} request accepted. Waiting for the
              resource and runtime result.
            </p>
          )}
          {activeOperation && (
            <RemoteMachineOperationCard
              key={activeOperation.id}
              operation={activeOperation}
              machine={machine}
              accounting={accounting}
              onRetryAccepted={() => void refreshProgress()}
              onReview={refreshProgress}
            />
          )}
          {operationQuery.error && (
            <ErrorBanner
              message={`Operation status unavailable. ${operationQuery.error.message}`}
              onRetry={() => void operationQuery.refetch()}
            />
          )}
        </section>
      )}
      <RemoteMachinePaidStart
        open={paidResumeOpen && remoteAdmission}
        onOpenChange={setPaidResumeOpen}
        machine={machine}
        onAccepted={() => {
          setAcceptedAction("resume");
          setTab("overview");
          void query.refetch();
        }}
      />
      <Tabs value={tab} onValueChange={setTab} className="min-w-0">
        <TabsList
          className="w-full justify-start"
          aria-label="Machine detail sections"
        >
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="access">Access</TabsTrigger>
          <TabsTrigger value="jobs">Jobs</TabsTrigger>
          <TabsTrigger value="confirmations">Confirmations</TabsTrigger>
          <TabsTrigger value="billing">Billing</TabsTrigger>
          <TabsTrigger value="timeline">Events</TabsTrigger>
          <TabsTrigger value="policy">Settings and rules</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-6 space-y-6">
          <section
            aria-label="Independent machine status"
            className="grid min-w-0 gap-4 sm:grid-cols-3"
          >
            <div className="min-w-0 space-y-1">
              <p className="text-11 text-muted-foreground">Execution</p>
              <p className="text-13 font-semibold">
                {machine.authorization_state === "active" && canPause
                  ? "Runtime authorized"
                  : "Execution unavailable"}
              </p>
              <p className="text-11 text-muted-foreground">
                Agent and external grants are checked separately.
              </p>
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-11 text-muted-foreground">Runtime billing</p>
              <p className="text-13 font-semibold">{accounting.label}</p>
              <p className="text-11 text-muted-foreground">
                {accounting.detail}
              </p>
              <Button variant="link" onClick={() => setTab("billing")}>
                Review billing
              </Button>
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-11 text-muted-foreground">Node connection</p>
              <p className="text-13 font-semibold">
                {node
                  ? node.is_connected
                    ? "WSS connected"
                    : "WSS disconnected"
                  : "WSS status unknown"}
              </p>
              <p className="text-11 text-muted-foreground">
                Last heartbeat: {time(machine.last_heartbeat_at)}
              </p>
            </div>
          </section>
          <DetailSection title="Workspace and temporary storage">
            <DetailRow
              label="Workspace allocation"
              value={`${machine.profile.workspace_gib} GiB`}
            />
            <DetailRow
              label="Temporary capacity policy"
              value="No fixed capacity declared"
            />
            <RemoteMachineUsage machineId={machineId} />
            <details className="text-12">
              <summary className="cursor-pointer px-4 py-3 font-medium">
                Storage and recovery rules
              </summary>
              <div className="space-y-3 px-4 py-3 text-muted-foreground">
                <p>
                  Temporary storage is managed by the deployment environment.
                  Temporary data may be lost after pause or Pod recreation.
                </p>
                <p>
                  If a disk fills, writes and jobs may fail. Disk pressure can
                  evict the Pod. Use an authorized file or shell client to
                  remove unneeded files when available. This fixed profile
                  cannot be expanded; review recovery with the platform operator
                  before irreversible deletion.
                </p>
              </div>
            </details>
          </DetailSection>
          <DetailSection title="Using this machine">
            <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
              <p className="font-medium text-foreground">
                {machine.specialist_assignment_state
                  ? {
                      none: "No specialist assigned",
                      pending: "Specialist assignment is pending",
                      assigned: "Specialist assigned",
                      permissions_required:
                        "Specialist permissions need configuration",
                      failed: "Specialist assignment could not be completed",
                    }[machine.specialist_assignment_state]
                  : "Specialist assignment status is unavailable"}
              </p>
              <p>
                Agent permissions are configured separately. External OAuth and
                Agent Key tools remain limited to authorized Commands and Files.
                All granted clients share the OS user and workspace; file roots
                do not isolate shell commands.
              </p>
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => setTab("access")}>
                  Review agent access
                </Button>
                {desktopNode &&
                  (desktopUnavailable ? (
                    <Button disabled>Watch / Take over desktop</Button>
                  ) : (
                    <Button asChild>
                      <Link
                        to="/assistant/machines/$nodeId/desktop"
                        params={{ nodeId: desktopNode.id }}
                      >
                        Watch / Take over desktop
                      </Link>
                    </Button>
                  ))}
                {machine.specialist_id && (
                  <Button asChild>
                    <Link
                      to="/assistant/automations"
                      search={{ agent: machine.specialist_id }}
                    >
                      Open specialist settings
                    </Link>
                  </Button>
                )}
              </div>
              {desktopUnavailable ? (
                <p role="status">{desktopUnavailable}</p>
              ) : desktopNode && !desktopNode.machine?.computer_ready ? (
                <p role="status">Computer permissions or display unavailable</p>
              ) : null}
            </div>
          </DetailSection>
          <RemoteMachineTechnicalDetails machine={machine} />
        </TabsContent>
        <TabsContent value="access" className="mt-6 space-y-6">
          {accessPanel ?? (
            <RemoteMachineAgentAccess machineId={machineId} node={node} />
          )}
          <MachineGrantList machineId={machineId} />
        </TabsContent>
        <TabsContent value="jobs" className="mt-6">
          <RemoteMachineJobs machineId={machineId} />
        </TabsContent>
        <TabsContent value="confirmations" className="mt-6">
          <RemoteMachineConfirmations
            machineId={machineId}
            confirmationId={confirmationId}
          />
        </TabsContent>
        <TabsContent value="billing" className="mt-6 space-y-6">
          <RemoteMachineBilling
            machineId={machineId}
            billingState={machine.billing_state}
          />
          <RemoteMachineServiceUsage machineId={machineId} />
        </TabsContent>
        <TabsContent value="timeline" className="mt-6">
          <RemoteMachineTimeline
            machineId={machineId}
            onOperation={setSelectedOperationId}
          />
        </TabsContent>
        <TabsContent value="policy" className="mt-6 space-y-8">
          <RemoteMachinePolicyPanel machine={machine} policy={policy} />
          <RemoteMachineClientSetup />
          {!closed && (
            <Card className="border-destructive/40">
              <CardHeader>
                <CardTitle className="text-destructive">
                  Delete machine
                </CardTitle>
                <CardDescription>
                  Permanent workspace deletion. Review pending work and
                  accounting before continuing.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex justify-end">
                <Button
                  variant="destructive"
                  onClick={() => setReviewAction("delete")}
                  disabled={machine.resource_state === "deleting"}
                >
                  <Trash2 className="size-3" />
                  Delete machine
                </Button>
              </CardContent>
            </Card>
          )}
        </TabsContent>
      </Tabs>
      {selectedOperationId && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setSelectedOperationId(null);
          }}
        >
          <DialogContent className="max-h-[85dvh] overflow-y-auto">
            <DialogHeader>
              <DialogTitle>
                {selectedOperationId === machine.active_operation_id
                  ? "Current operation details"
                  : "Historical operation"}
              </DialogTitle>
              <DialogDescription>
                This read-only view does not change the current lifecycle
                operation.
              </DialogDescription>
            </DialogHeader>
            {selectedQuery.error ? (
              <ErrorBanner
                message={selectedQuery.error.message}
                onRetry={() => void selectedQuery.refetch()}
              />
            ) : selectedOperation ? (
              <RemoteMachineOperationCard
                key={selectedOperation.id}
                operation={selectedOperation}
              />
            ) : (
              <p role="status">Loading operation…</p>
            )}
          </DialogContent>
        </Dialog>
      )}
      {reviewAction && (reviewAction !== "resume" || remoteAdmission) && (
        <LifecycleActionDialog
          key={`${machineId}:${reviewAction}`}
          action={reviewAction}
          machine={machine}
          policy={policy}
          onClose={() => setReviewAction(null)}
          onAccepted={() => {
            setAcceptedAction(reviewAction);
            setReviewAction(null);
            setTab("overview");
            void query.refetch();
          }}
          onReload={() => void query.refetch()}
        />
      )}
    </div>
  );
}

function RemoteMachineTechnicalDetails({
  machine,
}: {
  readonly machine: RemoteMachine;
}) {
  return (
    <details className="rounded-xl border border-border/50 bg-card text-12">
      <summary className="cursor-pointer px-4 py-3 font-medium">
        Technical information and resource binding
      </summary>
      <DetailRow
        label="Machine / Node ID"
        value={machine.node_id}
        mono
        copyable
      />
      <DetailRow
        label="Resource state"
        value={readable(machine.resource_state)}
      />
      <DetailRow
        label="Billing state"
        value={readable(machine.billing_state)}
      />
      <DetailRow
        label="Authorization state"
        value={readable(machine.authorization_state)}
      />
      <DetailRow
        label="Stop reason"
        value={
          machine.stop_reason ? readable(machine.stop_reason) : "Not reported"
        }
      />
      <DetailRow
        label="Runtime generation"
        value={String(machine.runtime_generation)}
      />
      <DetailRow
        label="Credential epoch"
        value={String(machine.credential_epoch)}
      />
      <DetailRow label="Resource revision" value={String(machine.revision)} />
      <DetailRow label="Updated" value={time(machine.updated_at)} />
      {machine.runtime_run_id && (
        <DetailRow
          label="Runtime billing run"
          value={machine.runtime_run_id}
          mono
          copyable
        />
      )}
      {machine.funded_until && (
        <DetailRow
          label="Current funded deadline"
          value={time(machine.funded_until)}
        />
      )}
      <DetailRow
        label="Sandbox"
        value={`${machine.sandbox_namespace}/${machine.sandbox_name}`}
        mono
      />
      <DetailRow
        label="Sandbox UID"
        value={machine.sandbox_uid ?? "Not observed"}
        mono
      />
      <DetailRow
        label="Pod UID"
        value={machine.pod_uid ?? "Not observed"}
        mono
      />
      <DetailRow
        label="Workspace claim"
        value={machine.pvc_name ?? "Not observed"}
        mono
      />
      <DetailRow
        label="Workspace claim UID"
        value={machine.pvc_uid ?? "Not observed"}
        mono
      />
      <DetailRow label="Profile" value={machine.profile.profile_version} mono />
      <p className="px-4 py-3 text-muted-foreground">
        These observed identifiers do not prove physical runtime stop or storage
        erasure.
      </p>
    </details>
  );
}

function LifecycleActionDialog({
  action,
  machine,
  policy,
  onClose,
  onAccepted,
  onReload,
}: {
  readonly action: OwnerAction;
  readonly machine: RemoteMachine;
  readonly policy?: RemoteMachinePolicy;
  readonly onClose: () => void;
  readonly onAccepted: (id: string) => void;
  readonly onReload: () => void;
}) {
  const ownerId = useAuthStore((s) => s.user?.id ?? "");
  const mutation = useRemoteMachineAction();
  const scope = `${machine.node_id}:${action}`;
  const [review] = useState(() => {
    try {
      return {
        intent: ownerId ? readRemoteMachineIntent(ownerId, scope) : null,
        revision: machine.revision,
        error: null,
      };
    } catch (error) {
      return {
        intent: null,
        revision: machine.revision,
        error:
          error instanceof Error
            ? error.message
            : "Stored action could not be read.",
      };
    }
  });
  const [error, setError] = useState<string | null>(review.error);
  const [conflict, setConflict] = useState(false);
  const form = useAppForm<z.infer<typeof actionReviewSchema>>({
    resolver: zodResolver(actionReviewSchema),
    defaultValues: { acknowledged: false },
  });
  const reviewRevision =
    review.intent?.body.expected_revision ?? review.revision;
  const label =
    action === "delete"
      ? "Delete permanently"
      : action === "resume"
        ? "Resume machine"
        : "Pause machine";
  async function submit() {
    if (!ownerId) {
      setError("Sign in as the machine owner before managing this resource.");
      return;
    }
    const body = {
      action,
      expected_revision: review.revision,
      ...(action === "delete"
        ? {
            data_policy_version:
              policy?.data_policy_version ?? machine.data_policy_version,
            acknowledge_data_loss: true,
          }
        : {}),
      ...(action === "resume"
        ? {
            profile_version: machine.profile.profile_version,
            policy_version: machine.policy_version,
            data_policy_version: machine.data_policy_version,
            acknowledge_shared_environment: true,
          }
        : {}),
    };
    try {
      if (action === "resume") assertRemoteMachineAdmission(ownerId);
      setError(null);
      const intent = getRemoteMachineIntent(ownerId, scope, body);
      const result = await mutation.mutateAsync({
        machineId: machine.node_id,
        body: remoteMachineActionRequestSchema.parse(intent.body),
        idempotencyKey: intent.idempotencyKey,
      });
      clearRemoteMachineIntent(ownerId, scope, intent.idempotencyKey);
      onAccepted(result.operation.id);
    } catch (cause) {
      setConflict(cause instanceof ApiError && cause.status === 409);
      setError(
        cause instanceof Error
          ? cause.message
          : "The operation could not be accepted. Retry keeps the same reviewed request.",
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
          <DialogTitle>{label}</DialogTitle>
          <DialogDescription>
            Review resource revision {String(reviewRevision)}. Only the
            signed-in owner can authorize this lifecycle action.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
            <div className="space-y-3 text-12 text-muted-foreground">
              {action === "resume" && <FreeResumePreflight machine={machine} />}
              {action === "delete" ? (
                <>
                  <p>
                    This permanently removes the workspace and temporary runtime
                    data, including HOME, /tmp and other temporary runtime
                    files. There are no snapshots or recovery. Running work will
                    be stopped, and resource cleanup remains pending until
                    current Kubernetes observations confirm the original
                    Sandbox, Pod and workspace PVC are absent after cascade
                    cleanup. Underlying storage is handled by its reclaim
                    policy. Accounting progress is tracked separately.
                  </p>
                  <p>
                    The cleanup target is{" "}
                    {policy
                      ? `${policy.cleanup_target_hours} hours`
                      : "not reported"}
                    , not a guaranteed erasure deadline. Submitting deletion
                    does not mean all data has been physically erased. A durable
                    tombstone and necessary evidence remain. A free slot is not
                    released until finalization.
                  </p>
                  <p>
                    Deletion immediately denies further output reads. Output is
                    never archived and may disappear before its one-hour maximum
                    after completion. Terminal job metadata remains for{" "}
                    {policy
                      ? `${policy.job_history_days} days`
                      : "the published retention period"}
                    , and terminal operation history for{" "}
                    {policy
                      ? `${policy.operation_history_days} days`
                      : "the published retention period"}
                    . Required audit and financial evidence is retained
                    separately.
                  </p>
                </>
              ) : action === "pause" ? (
                <>
                  <p>
                    Pause stops the compute Pod and retains the same workspace
                    and Machine identity. Completion is confirmed only after
                    current Sandbox and Pod observations. Paused storage is free
                    and remains until you choose deletion.
                  </p>
                  <p>
                    Resume starts a new runtime using the retained workspace; it
                    does not restore processes, memory or old jobs. An uncertain
                    stop remains pending for retry or operator review. A later
                    top-up never resumes the machine automatically.
                  </p>
                </>
              ) : (
                <>
                  <p>
                    Resume this free machine only after live eligibility and
                    current Kubernetes lifecycle checks. A new runtime uses the
                    retained workspace without restoring old processes or jobs.
                    It will never silently switch to paid.
                  </p>
                  <p>
                    Retained profile: {machine.profile.profile_version}. Policy:{" "}
                    {machine.policy_version}. Data policy:{" "}
                    {machine.data_policy_version}. Resume preserves the saved
                    specification and workspace.
                  </p>
                  <p>
                    Temporary storage has no fixed capacity commitment.{" "}
                    Temporary data may be lost after pause or Pod recreation.
                    All granted clients share the OS user and workspace. File
                    roots do not isolate shell commands.
                  </p>
                </>
              )}
            </div>
            <FormField
              control={form.control}
              name="acknowledged"
              render={({ field }) => (
                <FormItem>
                  <div className="flex items-start gap-2">
                    <FormControl>
                      <Checkbox
                        checked={field.value}
                        onCheckedChange={(checked) =>
                          field.onChange(checked === true)
                        }
                      />
                    </FormControl>
                    <FormLabel className="text-12">
                      I understand{" "}
                      {action === "delete"
                        ? "the irreversible data loss and cleanup policy"
                        : action === "pause"
                          ? "that work stops and the workspace is retained"
                          : "the current policies and shared environment risk"}
                      .
                    </FormLabel>
                  </div>
                  <FormMessage />
                </FormItem>
              )}
            />
            {review.intent && (
              <p className="text-12 text-muted-foreground">
                A previous request is awaiting a confirmed result. Retrying uses
                its original body and idempotency key.
              </p>
            )}
            {error && (
              <div role="alert">
                <ErrorBanner message={error} />
              </div>
            )}
            {review.error && (
              <div className="space-y-2 text-12 text-muted-foreground">
                <p>
                  Review the current machine and operation state before
                  discarding an unreadable request. Its previous outcome may be
                  unknown.
                </p>
                <Button
                  type="button"
                  onClick={() => {
                    clearRemoteMachineIntent(ownerId, scope);
                    onReload();
                    onClose();
                  }}
                >
                  Reset unreadable saved action and review
                </Button>
              </div>
            )}
            {conflict && (
              <Button
                type="button"
                onClick={() => {
                  clearRemoteMachineIntent(ownerId, scope);
                  onReload();
                  onClose();
                }}
              >
                Reload and review a new action
              </Button>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={onClose}
                disabled={mutation.isPending}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant={action === "delete" ? "destructive" : "primary"}
                disabled={
                  mutation.isPending ||
                  !form.watch("acknowledged") ||
                  conflict ||
                  Boolean(review.error)
                }
                isLoading={mutation.isPending}
              >
                {label}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function FreeResumePreflight({ machine }: { readonly machine: RemoteMachine }) {
  const machineId = machine.node_id;
  const profileVersion = machine.profile.profile_version;
  const quote = useRemoteMachineQuote();
  const request = quote.mutate;
  useEffect(() => {
    request({
      free_slot: true,
      machine_id: machineId,
      profile_version: profileVersion,
    });
  }, [request, machineId, profileVersion]);
  return (
    <section aria-label="Retained workspace resume preflight">
      {quote.error && (
        <p>
          Resume preflight is unavailable. Submitting repeats current checks; a
          saved accepted request can still recover its result.
        </p>
      )}
      <RemoteMachinePlatformReadiness
        platform={!quote.error ? quote.data?.platform : undefined}
      />
      <RemoteMachineQuota
        quota={!quote.error ? quote.data?.quota : undefined}
      />
    </section>
  );
}

function RemoteMachineTimeline({
  machineId,
  onOperation,
}: {
  readonly machineId: string;
  readonly onOperation: (id: string) => void;
}) {
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);
  const [pages, setPages] = useState<{
    machineId: string;
    cursors: (string | undefined)[];
  }>({ machineId, cursors: [undefined] });
  if (pages.machineId !== machineId)
    setPages({ machineId, cursors: [undefined] });
  const cursor =
    pages.machineId === machineId ? pages.cursors.at(-1) : undefined;
  const query = useRemoteMachineEvents(machineId, cursor);
  const events = query.error ? [] : (query.data?.events ?? []);
  const uniqueEvents = events.filter(
    (event, index) => events.findIndex((row) => row.id === event.id) === index,
  );
  const selectedEvent = uniqueEvents.find(
    (event) => event.id === selectedEventId,
  );
  return (
    <div className="space-y-4">
      <p className="text-12 text-muted-foreground">
        Safe lifecycle events only. This timeline contains no commands,
        environment values, file contents or job output. Terminal projections
        remain for 90 days; required active work, cleanup, idempotency and
        billing evidence are retained separately.
      </p>
      {query.isPending && (
        <p role="status" className="text-12">
          Loading operation timeline…
        </p>
      )}
      {query.error && (
        <ErrorBanner
          message={`Timeline unavailable. ${query.error.message}`}
          onRetry={() => void query.refetch()}
        />
      )}
      {!query.isPending && !query.error && !uniqueEvents.length && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
          No lifecycle events are available on this page.
        </p>
      )}
      <ol className="space-y-3">
        {uniqueEvents.map((event) => (
          <li
            key={event.id}
            className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-13 font-semibold">
                {readable(event.kind)}
              </span>
              <time
                className="text-11 text-text-tertiary"
                dateTime={event.created_at}
              >
                {time(event.created_at)}
              </time>
            </div>
            <p className="text-12 text-muted-foreground">{event.message}</p>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-11 text-text-tertiary">
                Responsibility: {event.responsibility}
              </span>
              <Button
                variant="ghost"
                onClick={() => setSelectedEventId(event.id)}
              >
                Event details
              </Button>
            </div>
          </li>
        ))}
      </ol>
      <div className="space-y-2 text-12 text-muted-foreground">
        <p>
          Loaded {uniqueEvents.length}{" "}
          {uniqueEvents.length === 1 ? "event" : "events"} on this page
        </p>
        <p>
          {query.data?.next_cursor === undefined
            ? " Older-history availability is unknown on this server."
            : " The server cursor preserves events with matching timestamps."}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={pages.cursors.length < 2 || query.isFetching}
            onClick={() =>
              setPages({ machineId, cursors: pages.cursors.slice(0, -1) })
            }
          >
            Newer events
          </Button>
          <Button
            disabled={!query.data?.next_cursor || query.isFetching}
            onClick={() => {
              if (query.data?.next_cursor)
                setPages({
                  machineId,
                  cursors: [...pages.cursors, query.data.next_cursor],
                });
            }}
          >
            Older events
          </Button>
          <Button
            onClick={() => {
              setPages({ machineId, cursors: [undefined] });
              void query.refetch();
            }}
          >
            Refresh timeline
          </Button>
        </div>
      </div>
      {selectedEvent && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setSelectedEventId(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Event details</DialogTitle>
              <DialogDescription>{selectedEvent.message}</DialogDescription>
            </DialogHeader>
            <DetailRow label="Event type" value={selectedEvent.kind} />
            <DetailRow
              label="Event time"
              value={time(selectedEvent.created_at)}
            />
            <DetailRow
              label="Responsibility"
              value={selectedEvent.responsibility}
            />
            <DetailRow
              label="Operation ID"
              value={selectedEvent.operation_id}
              mono
              copyable
            />
            <DialogFooter>
              <Button
                onClick={() => {
                  setSelectedEventId(null);
                  onOperation(selectedEvent.operation_id);
                }}
              >
                View operation
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}

function RemoteMachinePolicyPanel({
  machine,
  policy,
}: {
  readonly machine: RemoteMachine;
  readonly policy?: RemoteMachinePolicy;
}) {
  const owner = useAuthStore((state) => state.user?.id);
  const nodeQuery = useNode(machine.node_id);
  const node =
    !nodeQuery.error &&
    nodeQuery.data?.id === machine.node_id &&
    nodeQuery.data.owner.kind === "user" &&
    nodeQuery.data.owner.id === owner
      ? nodeQuery.data
      : undefined;
  return (
    <div className="space-y-8">
      <DetailSection title="Confirmation policy">
        {nodeQuery.error ? (
          <ErrorBanner
            message={`Confirmation policy unavailable. ${nodeQuery.error.message}`}
            onRetry={() => void nodeQuery.refetch()}
          />
        ) : (
          <DetailRow
            label="Node confirmation policy"
            value={
              node?.machine_confirm
                ? {
                    none: "No machine confirmation",
                    changes: "Confirm changes",
                    all: "Confirm every operation",
                  }[node.machine_confirm]
                : nodeQuery.isPending
                  ? "Loading confirmation policy…"
                  : "Not reported"
            }
          />
        )}
        <p className="px-4 py-3 text-12 text-muted-foreground">
          Read-only settings from the Node detail. Agent access mode and source
          policies may require additional confirmation; a single setting does
          not authorize execution.
        </p>
      </DetailSection>
      <DetailSection title="Fixed profile and limits">
        <DetailRow
          label="CPU"
          value={`${machine.profile.cpu_millis / 1000} vCPU`}
        />
        <DetailRow
          label="Memory"
          value={`${machine.profile.memory_mib / 1024} GiB`}
        />
        <DetailRow
          label="Persistent workspace"
          value={`${machine.profile.workspace_gib} GiB`}
        />
        <DetailRow
          label="Temporary storage"
          value="No fixed capacity declared"
        />
        <DetailRow
          label="Concurrent commands"
          value={policy ? String(policy.concurrent_jobs) : "Not reported"}
        />
        <DetailRow
          label="Command timeout"
          value={
            policy
              ? `${policy.default_command_timeout_seconds}s default · ${policy.maximum_command_timeout_seconds}s maximum`
              : "Not reported"
          }
        />
        <DetailRow
          label="Output limits"
          value={
            policy
              ? `${policy.job_output_bytes.toLocaleString()} bytes per job · ${policy.machine_output_cache_bytes.toLocaleString()} bytes machine cache`
              : "Not reported"
          }
        />
        <DetailRow
          label="Command input"
          value="32 KiB command · 64 KiB stdin"
        />
        <p className="px-4 py-3 text-12 text-muted-foreground">
          All subjects and child credentials share the command slots. There is
          no persistent queue; capacity rejection does not mean work is queued.
          File reads are paged at up to 4 KiB. Exact edits handle at most 16 MiB
          for both the original and result; this is not a universal file-size or
          write-request limit. Smaller gateway and protocol limits still apply.
          A command can stop before its requested timeout when authentication,
          grant, execution lease, funding or runtime authority expires.
        </p>
      </DetailSection>
      <DetailSection title="Network, environment and sharing">
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>
            Full Machine image includes Chromium, desktop and computer tooling.
            Only the selected capabilities are enabled; agent grants and live
            runtime readiness determine use. Only the human owner can watch or
            take over the desktop. Treat visible browser and desktop content as
            sensitive. External OAuth and Agent Key tools remain limited to
            authorized Commands and Files.
          </p>
          <p>
            This profile has no PTY, inbound ports, public IP or port mapping.
            It does not offer custom images, upgrades, autoscaling, exports or
            snapshots. This personal-account profile cannot be transferred to an
            organization or charged to an organization wallet.
          </p>
          <p>
            All authorized clients share the Machine and workspace. Commands and
            file tools use the shared command OS user. Authorized agents also
            share browser sessions in the existing shared mode. File roots
            constrain file tools and working directories; they do not isolate
            shell authority or secrets between clients.
          </p>
          <p>
            Only /workspace persists; HOME, /tmp, browser profiles and downloads
            are temporary unless safely saved in the workspace. Temporary
            storage is managed by the deployment environment; new Machines have
            no fixed temporary capacity commitment. Temporary data may be lost
            after pause or Pod recreation. A new runtime does not restore
            browser sessions or running processes.
          </p>
          <p>
            There is no persistent environment editor or secret manager. Job
            environment values are temporary. Downstream credentials stay in
            NyxID; each job must declare its services and remains bounded by its
            principal’s live service access and Machine service ceiling.
          </p>
        </div>
      </DetailSection>
      <DetailSection title="Retention and charging policy">
        <DetailRow label="Policy version" value={machine.policy_version} mono />
        <DetailRow
          label="Data policy version"
          value={machine.data_policy_version}
          mono
        />
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>
            Job metadata retention:{" "}
            {policy ? `${policy.job_history_days} days` : "not reported"};
            terminal operation retention:{" "}
            {policy ? `${policy.operation_history_days} days` : "not reported"}.
            Output is read only from the currently authorized live node,
            retained at most one hour after a job finishes, and can disappear
            earlier after cache eviction or runtime loss.
          </p>
          <p>
            Deletion confirms Sandbox and related namespace object cleanup,
            terminal creation responsibilities and accounting. Underlying
            storage follows its reclaim policy; physical storage release or
            erasure is not verified. Cleanup target:{" "}
            {policy ? `${policy.cleanup_target_hours} hours` : "not reported"}.
            A target is not a completion guarantee.
          </p>
          <p>
            Paused workspaces have no automatic expiry and no storage fee. A
            free machine never implies free downstream services. Runtime and
            gateway service charges are separate.
          </p>
          <p>
            Paid runs require an accepted rate and account-balance funding, with
            no per-instance cap. Account funds and runtime estimates are shared
            with other Machines and services, and do not promise a duration.
            Manual resume requires a new accepted quote after earlier liability
            is closed.
          </p>
        </div>
      </DetailSection>
    </div>
  );
}

function RemoteMachineClientSetup() {
  const config = usePublicConfig();
  const url = config.data?.mcp_url;
  return (
    <DetailSection title="Connect existing clients">
      <div className="space-y-4 px-4 py-3 text-12 text-muted-foreground">
        <p>
          Use the same NyxID HTTP MCP endpoint as your existing services. Sign
          in using the existing OAuth flow, then review the Machine selection.
          For an Agent Key, manage Machine access on its parent Key detail page;
          child login credentials inherit that shared grant. No separate
          principal registration is required.
        </p>
        {!url ? (
          <p role="status">
            The server MCP URL is unavailable. Open Settings → MCP when
            configuration is available.
          </p>
        ) : (
          <>
            {(
              [
                ["Claude Code", ".mcp.json", buildClaudeCodeConfig(url)],
                ["Codex", "~/.codex/config.toml", buildCodexConfig(url)],
              ] as const
            ).map(([name, path, value]) => (
              <div key={name} className="space-y-2">
                <p className="font-medium text-foreground">
                  {name} · {path}
                </p>
                <div className="relative">
                  <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3 pr-12 font-mono text-11 leading-relaxed">
                    {value}
                  </pre>
                  <Button
                    size="icon"
                    variant="ghost"
                    className="absolute right-2 top-2"
                    aria-label={`Copy ${name} HTTP MCP config`}
                    onClick={() =>
                      void copyToClipboard(value).then(
                        () => toast.success(`${name} config copied`),
                        () => toast.error("Could not copy config"),
                      )
                    }
                  >
                    <Copy className="size-3" />
                  </Button>
                </div>
              </div>
            ))}
          </>
        )}
        <p>
          Authenticate, discover tools, and use <code>nyx__machine_list</code>{" "}
          to inspect granted machines before execution. Tool availability still
          depends on the server’s external authority protocol; configuration
          alone does not grant access.
        </p>
        <p className="flex gap-2">
          <AlertTriangle className="size-4 shrink-0 text-warning" />
          Do not paste tokens, credentials or job environment values into these
          configuration examples or chat.
        </p>
      </div>
    </DetailSection>
  );
}

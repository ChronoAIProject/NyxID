import { remoteMachineReportedProfile } from "@/lib/remote-machine-presentation";
import { remoteMachineActionEligible, remoteMachineLifecyclePending } from "@/lib/remote-machine-lifecycle";
import {
  assertRemoteMachineAdmission,
  useRemoteMachineAdmission,
} from "@/hooks/use-remote-machine-admission";
import { RemoteMachineServiceUsage } from "@/components/machines/remote-machine-service-usage";
import { useEffect, useRef, useState } from "react";
import { RemoteMachineQuota } from "./remote-machine-preflight";
import { RemoteMachinePlatformReadiness } from "./remote-machine-platform-readiness";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  AlertTriangle,
  Copy,
  FolderCheck,
  Pause,
  Play,
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
  RemoteMachineProgress,
  type MachineAccountingView,
} from "./remote-machine-lifecycle";
export { RemoteMachineOperationCard } from "./remote-machine-lifecycle";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useAuthStore } from "@/stores/auth-store";
import { useRemoteMachineContinuity } from "@/hooks/use-remote-machine-continuity";
import { remoteMachineActionRequestSchema } from "@/schemas/machines";
import type { RemoteMachine, RemoteMachinePolicy } from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { ApiError } from "@/lib/api-client";
import {
  clearRemoteMachineIntent,
  acceptRemoteMachineIntent,
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
import { MachineGrantList } from "@/components/shared/machine-grant-list";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { RemoteMachineBilling } from "./remote-machine-billing";
import { RemoteMachineConfirmations } from "./remote-machine-confirmations";
import { RemoteMachinePaidStart } from "./remote-machine-paid-start";
import { RemoteMachineUsage } from "./remote-machine-usage";
import { MachineSettingsEditor } from "@/components/shared/machine-settings-editor";
import { SettingsSection } from "@/components/shared/machine-settings-section";
import { MachineApplicationStatus } from "@/components/assistant/machine-application-status";

const actionReviewSchema = z.object({ acknowledged: z.boolean() });
type OwnerAction = "pause" | "resume" | "delete";
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

const specialistAssignmentMessages: Record<
  RemoteMachine["specialist_assignment_state"],
  string | undefined
> = {
  none: undefined,
  pending: "Specialist assignment is pending",
  assigned: undefined,
  not_assigned: "Specialist is no longer assigned",
  failed: "Specialist assignment could not be completed",
};

type RemoteMachineDetailProps = {
  readonly machineId: string;
  readonly initialMachine?: RemoteMachine;
  readonly node?: NodeInfo;
  readonly policy?: RemoteMachinePolicy;
  readonly onClose?: () => void;
  readonly operationId?: string;
  readonly confirmationId?: string;
  readonly initialAgentId?: string;
  readonly accessPanel?: React.ReactNode;
  readonly initialAction?: OwnerAction;
  readonly onActivity?: () => void;
  readonly onLogins?: () => void;
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
  initialAgentId,
  accessPanel,
  initialAction,
  onActivity,
  onLogins,
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
  const { operationId, acceptedAction, machineReadBehind, readError: continuityError } =
    useRemoteMachineContinuity(machineId, machine);
  const operationQuery = useRemoteMachineOperation(operationId);
  const activeOperation =
    !operationQuery.error &&
    operationQuery.data?.id === operationId &&
    operationQuery.data?.machine_id === machineId
      ? operationQuery.data
      : undefined;
  const [selectedOperationId, setSelectedOperationId] = useState<string | null>(
    linkedOperationId ?? null,
  );
  const selectedQuery = useRemoteMachineOperation(
    selectedOperationId && selectedOperationId !== operationId
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
  const [tab, setTab] = useState(
    confirmationId ? "confirmations" : initialAgentId ? "access" : "overview",
  );
  const [linkedConfirmation, setLinkedConfirmation] = useState(confirmationId);
  if (linkedConfirmation !== confirmationId) {
    setLinkedConfirmation(confirmationId);
    if (confirmationId) setTab("confirmations");
  }
  const [reviewAction, setReviewAction] = useState<OwnerAction | null>(
    initialAction ?? null,
  );
  const [paidResumeOpen, setPaidResumeOpen] = useState(false);
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
          event.kind !== "completed" &&
          (!activeOperation || event.operation_id === activeOperation.id) &&
          !(
            event.kind === "accepted" &&
            activeOperation &&
            ["succeeded", "failed"].includes(activeOperation.state)
          ),
      )
    : undefined;

  async function refreshProgress() {
    const current = await query.refetch();
    if (
      useAuthStore.getState().user?.id !== userId ||
      current.error ||
      current.data?.node_id !== machineId
    )
      return false;
    if (!current.data.active_operation_id) return true;
    // A newly discovered operation first mounts its existing query observer.
    // Wait for that read before allowing a manual replay of a saved request.
    if (current.data.active_operation_id !== operationId) return false;
    const observed = await operationQuery.refetch();
    return Boolean(
      useAuthStore.getState().user?.id === userId &&
      !observed.error &&
      observed.data?.id === current.data.active_operation_id &&
      observed.data.machine_id === machineId,
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
  const canResume = remoteMachineActionEligible(machine.resource_state, "resume");
  const canPause = remoteMachineActionEligible(machine.resource_state, "pause");
  const lifecyclePending = remoteMachineLifecyclePending(
    machine, operationId, activeOperation, machineReadBehind,
  );
  const lifecycleAction = activeOperation?.action ?? acceptedAction;
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

  const presentation = machine.presentation;
  const reported = presentation
    ? remoteMachineReportedProfile(presentation)
    : node?.machine;
  const connected = Boolean(
    canPause &&
    machine.authorization_state === "active" &&
    (presentation ? presentation.connected : node?.is_connected),
  );
  const recoverableActions = (["pause", "resume", "delete"] as const).filter(
    (action) => {
      if (!userId || (action === "resume" && !remoteAdmission)) return false;
      try {
        return Boolean(
          readRemoteMachineIntent(userId, `${machineId}:${action}`),
        );
      } catch {
        return true;
      }
    },
  );
  const needsStatus = Boolean(
    title ||
    notice ||
    lifecyclePending ||
    activeOperation?.state === "failed" ||
    continuityError ||
    recoverableActions.length,
  );
  const showProgress = Boolean(
    lifecyclePending ||
    activeOperation?.state === "failed" ||
    continuityError ||
    operationQuery.error,
  );
  const progress = (
    <section aria-label="Current attention" className="space-y-3">
      {showProgress && (
        <RemoteMachineProgress
          key={
            operationId ??
            acceptedAction ??
            machine.node_id
          }
          machine={machine}
          operation={activeOperation}
          acceptedAction={lifecyclePending ? acceptedAction : null}
          readError={
            continuityError ?? (operationQuery.error
              ? `Operation status unavailable. ${operationQuery.error.message}`
              : undefined)
          }
          accounting={accounting}
          onCheck={refreshProgress}
          onActivity={onActivity}
        />
      )}
      {!showProgress && title && (
        <div className="space-y-1">
          <h3 className="text-13 font-semibold">{title}</h3>
          <p className="text-muted-foreground">{attention}</p>
        </div>
      )}
      {!showProgress && notice && (
        <p>
          {notice.message}{" "}
          <button
            type="button"
            className="text-primary-text underline"
            onClick={() => setSelectedOperationId(notice.operation_id)}
          >
            View operation {notice.operation_id}
          </button>
        </p>
      )}
      {!reported && (
        <div
          role="group"
          aria-label="Enabled capability badges"
          className="flex flex-wrap gap-2"
        >
          {machine.capabilities?.length ? (
            machine.capabilities.map((capability) => (
              <Badge key={capability} variant="secondary">
                {
                  {
                    shell: "Commands",
                    files: "Files",
                    browser: "Browser",
                    computer: "Computer",
                  }[capability]
                }
              </Badge>
            ))
          ) : (
            <span>Capabilities not reported</span>
          )}
        </div>
      )}
      {recoverableActions.map((action) => (
        <div key={action} className="space-y-2">
          <p role="status">
            A previous {action} request has an unconfirmed result. Check its
            status before retrying.
          </p>
          <Button
            type="button"
            variant="outline"
            onClick={() =>
              action === "resume" && !freeSlot
                ? setPaidResumeOpen(true)
                : setReviewAction(action)
            }
          >
            Recover {action} request
          </Button>
        </div>
      ))}
      <div className="flex flex-wrap gap-2">
        {!showProgress && needsStatus && (
          <Button
            type="button"
            variant="outline"
            onClick={() => void refreshProgress()}
          >
            Check status
          </Button>
        )}
        {!showProgress && onActivity && (
          <Button type="button" variant="outline" onClick={onActivity}>
            View activity
          </Button>
        )}
      </div>
      <RemoteMachineTechnicalDetails machine={machine} />
    </section>
  );
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <h2 className="sr-only">{machine.name ?? machine.sandbox_name}</h2>
      <div className="shrink-0 space-y-2 px-5 pb-4 text-12">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={statusVariant(machine.resource_state)}>
            {machine.resource_state === "ready"
              ? "Running"
              : readable(machine.resource_state)}
          </Badge>
          {machine.resource_state === "paused" && !lifecyclePending && (
            <Badge variant="secondary">
              <FolderCheck className="mr-1 size-3" />
              Workspace kept
            </Badge>
          )}
          <span className="text-muted-foreground">NyxID hosted</span>
        </div>
        <div className="flex flex-wrap gap-2">
          {(canPause || machine.resource_state === "stopping") && (
            <Button
              disabled={!canPause || lifecyclePending}
              onClick={() => setReviewAction("pause")}
            >
              <Pause />
              {machine.resource_state === "stopping" ||
              (lifecyclePending && lifecycleAction === "pause")
                ? "Pausing…"
                : "Pause machine"}
            </Button>
          )}
          {(canResume || machine.resource_state === "resuming") && (
            <Button
              disabled={
                !remoteAdmission ||
                !canResume ||
                lifecyclePending
              }
              onClick={() =>
                freeSlot ? setReviewAction("resume") : setPaidResumeOpen(true)
              }
            >
              <Play />
              {machine.resource_state === "resuming" ||
              (lifecyclePending && lifecycleAction === "resume")
                ? "Resuming…"
                : freeSlot
                  ? "Resume machine"
                  : "Review quote to resume"}
            </Button>
          )}
        </div>
      </div>
      {closed ? (
        <div className="min-h-0 space-y-4 overflow-y-auto px-5 pb-5">
          <SettingsSection title="Machine deleted">
            <p>{attention}</p>
            <p>
              Account saved logins are kept.
              {freeSlot &&
                " Your free slot is available only once the current entitlement confirms it."}
            </p>
            {remoteAdmission && (
              <Button asChild>
                <Link to="/assistant/machines/new" search={{ where: "hosted" }}>
                  Create another machine
                </Link>
              </Button>
            )}
            {onActivity && <Button onClick={onActivity}>View activity</Button>}
          </SettingsSection>
          <details
            open={machine.cleanup_state === "pending"}
            className="rounded-lg border border-border p-3 text-12"
          >
            <summary className="cursor-pointer font-medium">
              Deletion details
            </summary>
            <div className="pt-3">{progress}</div>
          </details>
        </div>
      ) : (
        <MachineSettingsEditor
          machineId={machineId}
          name={machine.name ?? machine.sandbox_name}
          ownerId={userId}
          machine={reported}
          connected={connected}
          desktopReady={
            connected &&
            (presentation?.desktop_ready ?? reported?.computer_ready ?? false)
          }
          agentVersion={
            presentation
              ? presentation.agent_version
              : node?.metadata?.agent_version
          }
          confirmation={
            presentation
              ? presentation.owner_confirmation
              : node?.machine_confirm
          }
          allowSharedLogins={
            presentation
              ? presentation.allow_single_user_saved_logins
              : node?.allow_single_user_saved_logins
          }
          separationSupport={presentation?.separation_support}
          historical={presentation?.historical ?? !connected}
          readOnlyPolicy
          editable={canPause && !lifecyclePending}
          onLogins={onLogins}
          separationStatusContent={
            machine.machine_configuration?.state !== "applied" ||
            machine.machine_configuration.reason ? (
              <p role="status">
                Machine configuration is awaiting confirmation. Agent
                authorization and environment readiness are checked in each
                Agent’s settings.
              </p>
            ) : undefined
          }
          membershipStatusContent={
            machine.specialist_assignment_state &&
            !["none", "assigned"].includes(
              machine.specialist_assignment_state,
            ) ? (
              <p role="status">
                {
                  specialistAssignmentMessages[
                    machine.specialist_assignment_state
                  ]
                }
              </p>
            ) : undefined
          }
          beforeVersion={
            <SettingsSection
              title={freeSlot ? "Personal free slot" : "Runtime"}
            >
              <div className="flex flex-wrap justify-between gap-2">
                <span className="font-medium">
                  {machine.profile.cpu_millis / 1000} vCPU ·{" "}
                  {machine.profile.memory_mib / 1024} GiB ·{" "}
                  {machine.profile.workspace_gib} GiB workspace
                </span>
                <Badge variant="secondary">
                  {freeSlot ? "Free runtime" : accounting.label}
                </Badge>
              </div>
              <p className="text-11 text-muted-foreground">
                {freeSlot ? "1 of 1 in use · " : ""}
                {machine.resource_state === "paused"
                  ? freeSlot
                    ? "Still in use while paused. Workspace kept."
                    : "Workspace kept. Resume requires a new quote."
                  : machine.resource_state === "deleting"
                    ? "Released once deletion is confirmed."
                    : freeSlot
                      ? "This machine uses your free slot."
                      : "Runtime charges follow the accepted billing contract."}
              </p>
              <p className="text-11 text-muted-foreground">
                Model and external API usage may be billed separately.
              </p>
            </SettingsSection>
          }
          statusContent={progress}
          afterSettings={
            <>
              <details className="rounded-lg border border-border p-3">
                <summary className="cursor-pointer">
                  More machine details
                </summary>
                <div className="flex flex-wrap gap-2 pt-3">
                  {[
                    ...(!freeSlot ? ["billing"] : []),
                    "policy",
                    "external",
                  ].map((value) => (
                    <Button
                      key={value}
                      type="button"
                      variant="outline"
                      onClick={() => setTab(value)}
                    >
                      {
                        {
                          billing: "Billing",
                          policy: "Settings and rules",
                          external: "External client access",
                        }[value]
                      }
                    </Button>
                  ))}
                </div>
              </details>
              <section
                aria-label="Delete machine"
                className="space-y-3 rounded-xl border border-destructive/30 p-4"
              >
                <h3 className="text-13 font-medium">Delete machine</h3>
                <p className="text-muted-foreground">
                  Permanently remove this machine and its workspace. Account
                  saved logins are kept.
                </p>
                {machine.resource_state === "deleting" ? (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() =>
                      document
                        .getElementById("machine-status")
                        ?.scrollIntoView({ block: "start" })
                    }
                  >
                    View deletion status
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="destructive"
                    disabled={lifecyclePending}
                    onClick={() => setReviewAction("delete")}
                  >
                    <Trash2 />
                    Delete machine
                  </Button>
                )}
              </section>
            </>
          }
        />
      )}
      <RemoteMachinePaidStart
        open={paidResumeOpen && remoteAdmission}
        onOpenChange={setPaidResumeOpen}
        machine={machine}
        onCheck={refreshProgress}
        onAccepted={() => {
          void query.refetch();
        }}
      />
      {(["confirmations", "policy", "external"].includes(tab) ||
        (tab === "billing" && !freeSlot)) && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setTab("overview");
          }}
        >
          <DialogContent scrollMode="body">
            <DialogHeader>
              <DialogTitle>
                {
                  {
                    confirmations: "Confirmations",
                    billing: "Billing",
                    policy: "Settings and rules",
                    external: "External client access",
                  }[tab]
                }
              </DialogTitle>
              <DialogDescription>{machine.name}</DialogDescription>
            </DialogHeader>
            {tab === "confirmations" && (
              <RemoteMachineConfirmations
                machineId={machineId}
                confirmationId={confirmationId}
              />
            )}
            {tab === "billing" && (
              <>
                <RemoteMachineBilling
                  machineId={machineId}
                  billingState={machine.billing_state}
                />
                <RemoteMachineServiceUsage machineId={machineId} />
              </>
            )}
            {tab === "policy" && (
              <>
                <RemoteMachinePolicyPanel machine={machine} policy={policy} />
                <RemoteMachineUsage machineId={machineId} />
                <RemoteMachineClientSetup />
              </>
            )}
            {tab === "external" && <MachineGrantList machineId={machineId} />}
          </DialogContent>
        </Dialog>
      )}
      {accessPanel}
      {selectedOperationId && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setSelectedOperationId(null);
          }}
        >
          <DialogContent scrollMode="body">
            <DialogHeader>
              <DialogTitle>
                {selectedOperationId === operationId
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
              <RemoteMachineOperationCard operation={selectedOperation} />
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
            setReviewAction(null);
            void query.refetch();
          }}
          onCheck={refreshProgress}
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
        Technical details
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
      <MachineApplicationStatus
        configuration={machine.machine_configuration}
        configurationApplicability="hosted"
      />
      <DetailRow label="Profile" value={machine.profile.profile_version} mono />
      <DetailRow
        label="Temporary capacity policy"
        value="No fixed capacity declared"
      />
      <RemoteMachineUsage machineId={machine.node_id} />
      <p className="px-4 py-3 text-muted-foreground">
        Temporary storage is managed by the deployment environment. Temporary
        data may be lost after pause or Pod recreation. If a disk fills, writes
        and jobs may fail. Disk pressure can evict the Pod. Use an authorized
        file or shell client to remove unneeded files when available. This fixed
        profile cannot be expanded.
      </p>
      <p className="px-4 py-3 text-muted-foreground">
        Shared legacy agents and external clients share their configured OS user
        and workspace; file roots do not isolate shell commands.
      </p>
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
  onCheck,
}: {
  readonly action: OwnerAction;
  readonly machine: RemoteMachine;
  readonly policy?: RemoteMachinePolicy;
  readonly onClose: () => void;
  readonly onAccepted: (id: string) => void;
  readonly onCheck: () => Promise<boolean>;
}) {
  const ownerId = useAuthStore((s) => s.user?.id ?? "");
  const mutation = useRemoteMachineAction();
  const scope = `${machine.node_id}:${action}`;
  const [review] = useState(() => {
    try {
      const intent = ownerId ? readRemoteMachineIntent(ownerId, scope) : null;
      if (
        intent &&
        remoteMachineActionRequestSchema.parse(intent.body).action !== action
      )
        throw new Error(
          "The saved request does not match this action. Review it before continuing.",
        );
      return { intent, revision: machine.revision, error: null };
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
  const [unknown, setUnknown] = useState(Boolean(review.intent));
  const [checking, setChecking] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [hasRequest, setHasRequest] = useState(Boolean(review.intent));
  const inFlight = useRef(false);
  const form = useAppForm<z.infer<typeof actionReviewSchema>>({
    resolver: zodResolver(actionReviewSchema),
    defaultValues: {
      acknowledged: review.intent?.body.acknowledge_data_loss === true,
    },
  });
  const reviewRevision =
    review.intent?.body.expected_revision ?? review.revision;
  const label =
    action === "delete"
      ? "Delete machine"
      : action === "resume"
        ? "Resume machine"
        : "Pause machine";
  const busy = mutation.isPending || submitting || checking;
  // A saved request is an idempotent recovery after the existing status check,
  // not permission to create a new action from an unsupported resource state.
  const resourceEligible = hasRequest || remoteMachineActionEligible(machine.resource_state, action);
  async function check() {
    if (inFlight.current) return;
    inFlight.current = true;
    setChecking(true);
    try {
      const loaded = await onCheck();
      if (useAuthStore.getState().user?.id !== ownerId) return;
      if (loaded) {
        setUnknown(false);
        setError(null);
      } else {
        setError(
          "The current machine and operation could not both be confirmed. Check status again before retrying the original request.",
        );
      }
    } finally {
      inFlight.current = false;
      setChecking(false);
    }
  }
  async function submit() {
    if (
      inFlight.current ||
      busy ||
      unknown ||
      conflict ||
      review.error ||
      !resourceEligible ||
      (action === "delete" && !form.getValues("acknowledged"))
    )
      return;
    if (!ownerId || useAuthStore.getState().user?.id !== ownerId) return;
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
    inFlight.current = true;
    setSubmitting(true);
    try {
      if (action === "resume") assertRemoteMachineAdmission(ownerId);
      setError(null);
      const intent = getRemoteMachineIntent(ownerId, scope, body);
      setHasRequest(true);
      const result = await mutation.mutateAsync({
        machineId: machine.node_id,
        body: remoteMachineActionRequestSchema.parse(intent.body),
        idempotencyKey: intent.idempotencyKey,
      });
      if (useAuthStore.getState().user?.id !== ownerId) return;
      acceptRemoteMachineIntent(ownerId, machine.node_id, action, intent.idempotencyKey, result.operation.id);
      onAccepted(result.operation.id);
    } catch (cause) {
      if (useAuthStore.getState().user?.id !== ownerId) return;
      const rejected =
        cause instanceof ApiError &&
        cause.status >= 400 &&
        cause.status < 500 &&
        cause.status !== 408;
      // A definite rejection cannot be retried with a silently rebased revision.
      // A lost response keeps the exact intent until a fresh read and manual retry.
      setConflict(rejected);
      if (rejected) clearRemoteMachineIntent(ownerId, scope);
      setUnknown(!rejected);
      setError(
        cause instanceof Error
          ? cause.message
          : "The request result could not be confirmed.",
      );
    } finally {
      inFlight.current = false;
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
      <DialogContent
        scrollMode="body"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          document.getElementById("remote-lifecycle-cancel")?.focus();
        }}
      >
        <DialogHeader className="pr-6">
          <div className="mb-2 flex items-center gap-2 text-11 text-text-tertiary">
            {action === "delete" ? (
              <Trash2 className="size-3.5" />
            ) : action === "pause" ? (
              <Pause className="size-3.5" />
            ) : (
              <Play className="size-3.5" />
            )}
            Machine action
          </div>
          <DialogTitle>
            {action === "delete" ? "Delete machine?" : `${label}?`}
          </DialogTitle>
          <DialogDescription className="break-all text-12">
            {machine.name ?? machine.sandbox_name}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
            <div className="space-y-3 text-12 text-muted-foreground">
              {action === "delete" ? (
                <>
                  <div className="flex items-start gap-2.5 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-foreground">
                    <AlertTriangle className="mt-0.5 size-4 shrink-0 text-destructive" />
                    <p>
                      This permanently deletes the machine and its workspace.
                      This cannot be undone.
                    </p>
                  </div>
                  <p>
                    Running tasks will stop. Back up any files you want to keep
                    before continuing. Account saved logins are kept.
                  </p>
                  <p>
                    {(machine.free_slot ?? machine.billing_state === "free")
                      ? "Your free slot is released after deletion is confirmed."
                      : "Deletion completes after cleanup and runtime accounting are confirmed."}
                  </p>
                </>
              ) : action === "pause" ? (
                <>
                  <p className="flex items-center gap-2 text-foreground">
                    <FolderCheck className="size-4 shrink-0 text-success" />
                    Your workspace will be kept.
                  </p>
                  <ul className="list-disc space-y-2 pl-4">
                    <li>Active tasks and running processes will stop.</li>
                    <li>Temporary data and browser sessions may be lost.</li>
                    <li>
                      {(machine.free_slot ?? machine.billing_state === "free")
                        ? "This machine will continue to occupy your free slot."
                        : "Runtime billing stops after the platform confirms pause. Workspace storage is kept."}
                    </li>
                  </ul>
                </>
              ) : (
                <>
                  <div className="flex items-center justify-between gap-2 rounded-lg border border-border px-3 py-2.5 text-foreground">
                    <span>Personal free slot</span>
                    <Badge variant="success">Free runtime</Badge>
                  </div>
                  <p className="flex items-center gap-2 text-foreground">
                    <FolderCheck className="size-4 shrink-0 text-success" />
                    Your saved workspace will be available.
                  </p>
                  <p>
                    Previous jobs, running processes and browser sessions will
                    not be restored.
                  </p>
                  <p>Model and external API usage may be billed separately.</p>
                  <details>
                    <summary className="cursor-pointer">
                      Resume checks and policies
                    </summary>
                    <FreeResumePreflight machine={machine} />
                    <p>
                      Profile: {machine.profile.profile_version}. Policy:{" "}
                      {machine.policy_version}. Data policy:{" "}
                      {machine.data_policy_version}. Resume preserves the saved
                      specification and workspace.
                    </p>
                  </details>
                </>
              )}
            </div>
            {action === "delete" && (
              <FormField
                control={form.control}
                name="acknowledged"
                render={({ field }) => (
                  <FormItem>
                    <div className="flex items-start gap-2.5 rounded-lg border border-border p-3">
                      <FormControl>
                        <Checkbox
                          checked={field.value}
                          onCheckedChange={(checked) =>
                            field.onChange(checked === true)
                          }
                        />
                      </FormControl>
                      <FormLabel className="text-12">
                        I understand that this machine's workspace will be
                        permanently deleted.
                      </FormLabel>
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
            {unknown && (
              <div className="space-y-2 text-12">
                <p role="status">
                  The request result is unknown. It may have been accepted.
                  Check status before retrying the original reviewed request.
                </p>
                <Button
                  type="button"
                  variant="outline"
                  disabled={busy}
                  isLoading={checking}
                  onClick={() => void check()}
                >
                  Check request status
                </Button>
              </div>
            )}
            {error && (
              <div role="alert">
                <ErrorBanner message={error} />
              </div>
            )}
            {!resourceEligible && (
              <p role="status" className="text-12">
                This action is unavailable in the current machine state.
              </p>
            )}
            {(review.error || conflict) && (
              <div className="space-y-2 text-12">
                <p>
                  Reload the latest state and review a new action. The
                  previously reviewed request will not be changed automatically.
                </p>
                <Button
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    void (async () => {
                      if (
                        !(await onCheck()) ||
                        useAuthStore.getState().user?.id !== ownerId
                      ) {
                        setError(
                          "The latest state could not be read. Check status again before reviewing a new action.",
                        );
                        return;
                      }
                      clearRemoteMachineIntent(ownerId, scope);
                      onClose();
                    })();
                  }}
                >
                  {review.error
                    ? "Reset unreadable saved action and review"
                    : "Reload and review a new action"}
                </Button>
              </div>
            )}
            <details className="text-12 text-muted-foreground">
              <summary className="cursor-pointer">Action details</summary>
              <p>
                Reviewed resource revision: {String(reviewRevision)}. Retries
                keep the original request and idempotency key. Closing this
                dialog does not cancel an accepted request.
              </p>
            </details>
            <DialogFooter>
              <Button
                id="remote-lifecycle-cancel"
                type="button"
                variant="outline"
                onClick={onClose}
              >
                {submitting ? "Close · work continues" : "Cancel"}
              </Button>
              <Button
                type="submit"
                variant={action === "delete" ? "destructive" : "primary"}
                disabled={
                  busy ||
                  !resourceEligible ||
                  (action === "delete" && !form.watch("acknowledged")) ||
                  unknown ||
                  conflict ||
                  Boolean(review.error)
                }
                isLoading={submitting}
              >
                {action === "delete" ? (
                  <Trash2 />
                ) : action === "pause" ? (
                  <Pause />
                ) : (
                  <Play />
                )}
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

export function RemoteMachineTimeline({
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
            Sharing is set per Agent. Shared legacy Agents and external clients
            share their configured OS user and workspace. Shared legacy Agents
            also share browser sessions. Separated assignments require current
            support and their own workspace and browser profiles; they still
            share the machine kernel. File roots constrain file tools and
            working directories; they do not isolate shell authority or secrets
            between shared clients. Review each Agent’s mode in Access.
          </p>
          <p>
            The legacy /workspace and separated workspaces are retained on the
            original protected volume. Identity mappings, generations and
            tombstones are supervisor-protected and retained for recovery. HOME,
            /tmp, browser profiles and downloads are temporary; mode changes do
            not migrate shared files or copy cookies. Recovery failure keeps
            execution unavailable. Temporary storage is managed by the
            deployment environment; new Machines have no fixed temporary
            capacity commitment. Temporary data may be lost after pause or Pod
            recreation. A new runtime does not restore browser sessions or
            running processes.
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

import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

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
} from "@/hooks/use-machines";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useAuthStore } from "@/stores/auth-store";
import { remoteMachineActionRequestSchema } from "@/schemas/machines";
import type {
  RemoteMachine,
  RemoteMachineOperation,
  RemoteMachinePolicy,
} from "@/schemas/machines";
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
  return value ? new Date(value).toLocaleString(getFormattingLocale()) : translateUi("Not reported");
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

export function RemoteMachineDetail({
  machineId,
  initialMachine,
  node,
  policy,
  onClose,
  operationId: linkedOperationId,
  confirmationId,
  accessPanel,
}: {
  readonly machineId: string;
  readonly initialMachine?: RemoteMachine;
  readonly node?: NodeInfo;
  readonly policy?: RemoteMachinePolicy;
  readonly onClose?: () => void;
  readonly operationId?: string;
  readonly confirmationId?: string;
  readonly accessPanel?: React.ReactNode;
}) {
  const { t } = useUiTranslation();
  const userId = useAuthStore((state) => state.user?.id);
  const remoteAdmission = useRemoteMachineAdmission();
  const query = useRemoteMachine(machineId);
  const machine = query.error ? undefined : (query.data ?? initialMachine);
  const [reviewOwner, setReviewOwner] = useState(userId);
  const [reviewAction, setReviewAction] = useState<OwnerAction | null>(null);
  const [paidResumeOpen, setPaidResumeOpen] = useState(false);
  if (
    reviewOwner !== userId ||
    (!remoteAdmission && (reviewAction === "resume" || paidResumeOpen))
  ) {
    setReviewOwner(userId);
    setReviewAction(null);
    setPaidResumeOpen(false);
  }
  const [selectedOperationId, setSelectedOperationId] = useState<string | null>(
    linkedOperationId ?? null,
  );
  const operationId =
    selectedOperationId ?? machine?.active_operation_id ?? undefined;
  const operation = useRemoteMachineOperation(operationId);
  const [tab, setTab] = useState(confirmationId ? "confirmations" : "overview");
  if (query.isPending && !machine)
    return (
      <div
        role="status"
        aria-label={t("Loading remote machine")}
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
          message={query.error?.message ?? t("Remote machine not found.")}
          onRetry={() => void query.refetch()}
        />
      </div>
    );
  const transitioning = [
    "provisioning",
    "stopping",
    "resuming",
    "deleting",
    "reconciling",
  ].includes(machine.resource_state);
  const closed = machine.resource_state === "deleted";
  const freeSlot = machine.free_slot ?? machine.billing_state === "free";
  const canResume = machine.resource_state === "paused";
  const canPause = ["ready", "degraded"].includes(machine.resource_state);
  const desktopNode =
    node?.id === machine.node_id &&
    node.owner.kind === "user" &&
    node.owner.id === userId &&
    (node.machine?.browser || node.machine?.computer)
      ? node
      : undefined;
  const desktopUnavailable = desktopNode
    ? !desktopNode.is_connected
      ? "Desktop unavailable while this machine is disconnected."
      : !canPause || machine.authorization_state !== "active"
        ? "Desktop unavailable until this machine has an active runtime."
        : null
    : null;
  return (
    <div className="min-h-0 space-y-6 overflow-auto p-5">
      <PageHeader
        title={machine.name ?? node?.name ?? t("Remote Nyx Machine")}
        description={t("Owner controls for this hosted machine. Execution grants do not confer management access.")}
        actions={
          <Button
            size="icon"
            aria-label={t("Refresh machine")}
            onClick={() => void query.refetch()}
            disabled={query.isFetching}
          >
            <RefreshCw className="size-3" />
          </Button>
        }
      />
      <section
        aria-label={t("Hosted machine configuration")}
        className="space-y-3 rounded-xl border border-border bg-card p-4 text-12"
      >
        <p className="font-medium">{t("Enabled capabilities")}</p>
        <p>
          {machine.capabilities
            ?.map((capability) => capabilityLabels[capability])
            .join(" · ") ?? t("Configuration is unavailable from this server.")}
        </p>
        <p className="font-medium">
          {machine.specialist_assignment_state
            ? {
                none: "No specialist assigned",
                pending: "Specialist assignment is pending",
                assigned: "Specialist assigned",
                permissions_required:
                  "Specialist permissions need configuration",
                failed: "Specialist assignment could not be completed",
              }[machine.specialist_assignment_state]
            : t("Specialist assignment status is unavailable")}
        </p>
        {machine.specialist_assignment_state === "pending" ? (
          <p>{t("Assignment will be checked when this Machine is ready. Follow this Machine’s progress; creating another Machine does not resolve the pending assignment.")}</p>
        ) : null}
        {machine.specialist_assignment_state === "permissions_required" ||
        machine.specialist_assignment_state === "failed" ? (
          <p>{t("Review the existing specialist assignment and its allowed capabilities before use. This Machine has already been created.")}</p>
        ) : null}
        <p>{t("Enabled capabilities and specialist assignment do not authorize every caller. Agent permissions are configured separately. External OAuth and Agent Key tools remain limited to authorized Commands and Files.")}</p>
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => setTab("access")}>{t("Review agent access")}</Button>
          {desktopNode ? (
            desktopUnavailable ? (
              <Button disabled>{t("Watch / Take over desktop")}</Button>
            ) : (
              <Button asChild>
                <Link
                  to="/assistant/machines/$nodeId/desktop"
                  params={{ nodeId: desktopNode.id }}
                >{t("Watch / Take over desktop")}</Link>
              </Button>
            )
          ) : null}
          {machine.specialist_id ? (
            <Button asChild>
              <Link
                to="/assistant/automations"
                search={{ agent: machine.specialist_id }}
              >{t("Open specialist settings")}</Link>
            </Button>
          ) : null}
        </div>
        {desktopUnavailable ? (
          <p role="status" className="text-muted-foreground">
            {desktopUnavailable}
          </p>
        ) : desktopNode && !desktopNode.machine?.computer_ready ? (
          <p role="status" className="text-muted-foreground">{t("Computer permissions or display unavailable")}</p>
        ) : null}
      </section>
      {query.error && (
        <ErrorBanner
          message={t("The latest machine state could not be loaded. {{message}}", { message: translateErrorMessage(query.error.message) })}
          onRetry={() => void query.refetch()}
        />
      )}
      <div className="flex flex-wrap gap-2">
        <Badge variant={statusVariant(machine.resource_state)}>
          {readable(machine.resource_state)}
        </Badge>
        <Badge variant="secondary">
          {machine.profile.cpu_millis / 1000}{t(" vCPU ·")}{" "}
          {machine.profile.memory_mib / 1024}{t(" GiB RAM")}</Badge>
        <div
          role="group"
          aria-label={t("Enabled capability badges")}
          className="flex flex-wrap gap-2"
        >
          {machine.capabilities?.length ? (
            machine.capabilities.map((capability) => (
              <Badge key={capability} variant="secondary">
                {capabilityLabels[capability]}
              </Badge>
            ))
          ) : (
            <Badge variant="secondary">{t("Capabilities not reported")}</Badge>
          )}
        </div>
        <Badge variant="warning">{t("Shared environment")}</Badge>
      </div>
      {(machine.resource_state === "reconciling" ||
        machine.resource_state === "failed" ||
        machine.resource_state === "degraded") && (
        <div
          role="status"
          className="rounded-xl border border-warning/30 bg-warning/10 p-4 text-12 text-muted-foreground"
        >{t("The platform is checking resource and runtime evidence. A disconnected node does not prove the runtime has stopped. Review the operation below; retained workspace data is not deleted automatically.")}</div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          onClick={() => setReviewAction("pause")}
          disabled={!canPause || transitioning || Boolean(query.error)}
        >
          <Pause className="size-3" />{t("Pause machine")}</Button>
        <Button
          onClick={() =>
            freeSlot ? setReviewAction("resume") : setPaidResumeOpen(true)
          }
          disabled={
            !remoteAdmission ||
            !canResume ||
            transitioning ||
            Boolean(query.error)
          }
        >
          <Play className="size-3" />
          {freeSlot ? t("Resume machine") : t("Review quote to resume")}
        </Button>
        {onClose && (
          <Button variant="ghost" onClick={onClose}>{t("Back to machines")}</Button>
        )}
      </div>
      {!remoteAdmission && machine.resource_state === "paused" && (
        <p role="status" className="text-12 text-muted-foreground">{t("Remote Machines are not enabled for this account. Resume is unavailable.")}</p>
      )}
      {machine.resource_state === "paused" && !freeSlot && (
        <p role="status" className="text-12 text-muted-foreground">{t("Paid resume requires explicit acceptance of a new quote and a finite funding reservation. Adding funds never resumes a machine automatically. Any earlier pending liability must be resolved separately.")}</p>
      )}
      <RemoteMachinePaidStart
        open={paidResumeOpen && remoteAdmission && reviewOwner === userId}
        onOpenChange={setPaidResumeOpen}
        machine={machine}
        onAccepted={(_machineId, nextOperationId) => {
          setSelectedOperationId(nextOperationId);
          void query.refetch();
        }}
      />
      <RemoteMachineNotifications machineId={machineId} />
      {operation.data && (
        <RemoteMachineOperationCard operation={operation.data} />
      )}
      {operation.error && (
        <ErrorBanner
          message={t("Operation status unavailable. {{message}}", { message: translateErrorMessage(operation.error.message) })}
          onRetry={() => void operation.refetch()}
        />
      )}
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList className="justify-start">
          <TabsTrigger value="overview">{t("Overview")}</TabsTrigger>
          <TabsTrigger value="access">{t("Access")}</TabsTrigger>
          <TabsTrigger value="jobs">{t("Jobs")}</TabsTrigger>
          <TabsTrigger value="confirmations">{t("Confirmations")}</TabsTrigger>
          <TabsTrigger value="billing">{t("Billing")}</TabsTrigger>
          <TabsTrigger value="timeline">{t("Timeline")}</TabsTrigger>
          <TabsTrigger value="policy">{t("Policy and setup")}</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-6 space-y-8">
          <DetailSection title={t("Current state")}>
            <DetailRow
              label={t("Stop reason")}
              value={
                machine.stop_reason?.replaceAll("_", " ") ?? "Not reported"
              }
            />
            <DetailRow
              label={t("Resource state")}
              value={readable(machine.resource_state)}
              badge
              badgeVariant={statusVariant(machine.resource_state)}
            />
            <DetailRow
              label={t("Billing state")}
              value={readable(machine.billing_state)}
              badge
              badgeVariant={statusVariant(machine.billing_state)}
            />
            <DetailRow
              label={t("Authorization state")}
              value={readable(machine.authorization_state)}
              badge
              badgeVariant={statusVariant(machine.authorization_state)}
            />
            <DetailRow
              label={t("Node connection")}
              value={
                node
                  ? node.is_connected
                    ? "WSS connected"
                    : "WSS disconnected"
                  : "WSS status unknown"
              }
            />
            <DetailRow
              label={t("Last heartbeat")}
              value={time(machine.last_heartbeat_at)}
            />
            <DetailRow
              label={t("Runtime generation")}
              value={String(machine.runtime_generation)}
            />
            <DetailRow
              label={t("Credential epoch")}
              value={String(machine.credential_epoch)}
            />
            <DetailRow
              label={t("Resource revision")}
              value={String(machine.revision)}
            />
            <DetailRow label={t("Updated")} value={time(machine.updated_at)} />
            {machine.runtime_run_id && (
              <DetailRow
                label={t("Runtime billing run")}
                value={machine.runtime_run_id}
                mono
                copyable
              />
            )}
            {machine.funded_until && (
              <DetailRow
                label={t("Current funded deadline")}
                value={time(machine.funded_until)}
              />
            )}
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("Resource readiness, billing eligibility, and current execution authorization are independent. A ready resource alone does not allow any client to execute.")}</p>
          </DetailSection>
          <DetailSection title={t("Workspace and temporary storage")}>
            <DetailRow
              label={t("Workspace allocation")}
              value={`${machine.profile.workspace_gib} GiB`}
            />
            <DetailRow
              label={t("Temporary capacity policy")}
              value="No fixed capacity declared"
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("Temporary storage is managed by the deployment environment. No fixed capacity is promised. Temporary data may be lost after pause or Pod recreation.")}</p>
            <RemoteMachineUsage machineId={machineId} />
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("If a disk fills, writes and jobs may fail. Disk pressure can evict the Pod. Use an authorized file or shell client to remove unneeded files when the runtime is available, then retry the failed work. This fixed profile cannot be expanded. If recovery tools cannot run, review the operation and contact the platform operator before choosing irreversible deletion.")}</p>
          </DetailSection>
          <DetailSection title={t("Resource binding")}>
            <DetailRow
              label={t("Machine / Node ID")}
              value={machine.node_id}
              mono
              copyable
            />
            <DetailRow
              label={t("Sandbox")}
              value={`${machine.sandbox_namespace}/${machine.sandbox_name}`}
              mono
            />
            <DetailRow
              label={t("Sandbox UID")}
              value={machine.sandbox_uid ?? "Not observed"}
              mono
            />
            <DetailRow
              label={t("Pod UID")}
              value={machine.pod_uid ?? "Not observed"}
              mono
            />
            <DetailRow
              label={t("Workspace claim")}
              value={machine.pvc_name ?? "Not observed"}
              mono
            />
            <DetailRow
              label={t("Workspace claim UID")}
              value={machine.pvc_uid ?? "Not observed"}
              mono
            />
            <DetailRow
              label={t("Profile")}
              value={machine.profile.profile_version}
              mono
            />
            <p className="px-4 py-3 text-12 text-muted-foreground">{t("These identifiers describe the observed resource binding. They are not credentials or proof of physical runtime stop, writer fencing, or storage erasure.")}</p>
          </DetailSection>
          <DetailSection title={t("Access and client identity")}>
            <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
              <p>{t("NyxBot and specialists use their existing Agent assignments. External clients use the signed-in person plus an OAuth client, or the person plus a parent Agent Key. Children of one parent Key share that parent’s Machine grant.")}</p>
              <p>{t("Review capabilities, file roots, service ceilings, expiry and grant revisions in the existing Agent, Key detail, or granted-app controls. Revoking Machine access affects that grant; revoking an OAuth consent or parent Key affects its authority more broadly. Logging out one child credential is limited to that child.")}</p>
              <div className="flex flex-wrap gap-2">
                <Button asChild>
                  <Link to="/keys" search={{ tab: "api-keys" }}>{t("Agent Keys")}</Link>
                </Button>
                <Button asChild>
                  <Link to="/settings/consents">{t("Granted apps")}</Link>
                </Button>
              </div>
            </div>
          </DetailSection>
          <DetailSection title={t("Cleanup and retention")}>
            <DetailRow
              label={t("Cleanup state")}
              value={readable(machine.cleanup_state)}
              badge
              badgeVariant={statusVariant(machine.cleanup_state)}
            />
            <div className="space-y-2 px-4 py-3 text-12 text-muted-foreground">
              <p>{t("Paused workspaces remain until you explicitly delete the machine. Pausing has no storage fee.")}</p>
              <p>{t("Deletion removes the workspace and volume after Kubernetes resource cleanup is confirmed. Cleanup and accounting progress remain separate. The 24-hour cleanup target is not guaranteed erasure. The durable tombstone and required evidence remain; a free slot is released only after finalization.")}</p>
            </div>
          </DetailSection>
          <p className="text-12 text-muted-foreground">{t("All clients granted this machine share its OS user and workspace. File roots constrain file tools and working directories; roots do not isolate shell commands. Do not treat them as protection between clients.")}</p>
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
        <TabsContent value="billing" className="mt-6">
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
        </TabsContent>
      </Tabs>
      {!closed && (
        <Card className="border-destructive/40">
          <CardHeader>
            <CardTitle className="text-destructive">{t("Delete machine")}</CardTitle>
            <CardDescription>{t("Permanent workspace deletion. Review pending work and accounting before continuing.")}</CardDescription>
          </CardHeader>
          <CardContent className="flex justify-end">
            <Button
              variant="destructive"
              onClick={() => setReviewAction("delete")}
              disabled={
                machine.resource_state === "deleting" || Boolean(query.error)
              }
            >
              <Trash2 className="size-3" />{t("Delete machine")}</Button>
          </CardContent>
        </Card>
      )}
      {reviewAction &&
        reviewOwner === userId &&
        (reviewAction !== "resume" || remoteAdmission) && (
          <LifecycleActionDialog
            key={`${machineId}:${reviewAction}`}
            action={reviewAction}
            machine={machine}
            policy={policy}
            onClose={() => setReviewAction(null)}
            onAccepted={(id) => {
              setSelectedOperationId(id);
              setReviewAction(null);
              void query.refetch();
            }}
            onReload={() => void query.refetch()}
          />
        )}
    </div>
  );
}

export function RemoteMachineOperationCard({
  operation,
}: {
  readonly operation: RemoteMachineOperation;
}) {
  const { t } = useUiTranslation();
  return (
    <DetailSection
      title={t("{{value}} operation", { value: readable(operation.action) })}
      action={
        <Badge variant={statusVariant(operation.state)}>
          {readable(operation.state)}
        </Badge>
      }
    >
      <DetailRow label={t("Operation ID")} value={operation.id} mono copyable />
      <DetailRow label={t("Cleanup")} value={readable(operation.cleanup_state)} />
      <DetailRow
        label={t("Attempts after retry")}
        value={String(operation.retry_count)}
      />
      <DetailRow label={t("Created")} value={time(operation.created_at)} />
      <DetailRow label={t("Updated")} value={time(operation.updated_at)} />
      {operation.next_attempt_at && (
        <DetailRow
          label={t("Next platform attempt")}
          value={time(operation.next_attempt_at)}
        />
      )}
      {operation.deadline_at && (
        <DetailRow
          label={t("Investigation target")}
          value={time(operation.deadline_at)}
        />
      )}
      {operation.terminal_at && (
        <DetailRow
          label={t("Terminal result recorded")}
          value={time(operation.terminal_at)}
        />
      )}
      {operation.error_code && (
        <DetailRow label={t("Recovery code")} value={operation.error_code} mono />
      )}
      {operation.error_message && (
        <p role="alert" className="px-4 py-3 text-12 text-destructive">
          {operation.error_message}
        </p>
      )}
      {operation.evidence_digest && (
        <DetailRow
          label={t("Evidence reference")}
          value={operation.evidence_digest}
          mono
        />
      )}
      <p className="px-4 py-3 text-12 text-muted-foreground">{t("Operation status refreshes automatically. The operation ID is a reference only; owner authentication is required to view it.")}</p>
    </DetailSection>
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
  const { t } = useUiTranslation();
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
          <DialogDescription>{t("Review resource revision ")}{String(reviewRevision)}{t(". Only the signed-in owner can authorize this lifecycle action.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
            <div className="space-y-3 text-12 text-muted-foreground">
              {action === "resume" && <FreeResumePreflight machine={machine} />}
              {action === "delete" ? (
                <>
                  <p>{t("This permanently removes the workspace and temporary runtime data, including HOME, /tmp and other temporary runtime files. There are no snapshots or recovery. Running work will be stopped, and resource cleanup remains pending until current Kubernetes observations confirm the Sandbox, Pod, workspace PVC and required PV cleanup. Accounting progress is tracked separately.")}</p>
                  <p>{t("The cleanup target is 24 hours, not a guaranteed erasure deadline. Submitting deletion does not mean all data has been physically erased. A durable tombstone and necessary evidence remain. A free slot is not released until finalization.")}</p>
                  <p>{t("Deletion immediately denies further output reads. Output is never archived and may disappear before its one-hour maximum after completion. Terminal job metadata remains for 30 days, and terminal operation history for 90 days. Required audit and financial evidence is retained separately.")}</p>
                </>
              ) : action === "pause" ? (
                <>
                  <p>{t("Pause stops the compute Pod and retains the same workspace and Machine identity. Completion is confirmed only after current Sandbox and Pod observations. Paused storage is free and remains until you choose deletion.")}</p>
                  <p>{t("Resume starts a new runtime using the retained workspace; it does not restore processes, memory or old jobs. An uncertain stop remains pending for retry or operator review. A later top-up never resumes the machine automatically.")}</p>
                </>
              ) : (
                <>
                  <p>{t("Resume this free machine only after live eligibility and current Kubernetes lifecycle checks. A new runtime uses the retained workspace without restoring old processes or jobs. It will never silently switch to paid.")}</p>
                  <p>{t("Retained profile: ")}{machine.profile.profile_version}{t(". Policy:")}{" "}
                    {machine.policy_version}{t(". Data policy:")}{" "}
                    {machine.data_policy_version}{t(". Resume preserves the saved specification and workspace.")}</p>
                  <p>{t("Temporary storage has no fixed capacity commitment.")}{" "}{t("Temporary data may be lost after pause or Pod recreation. All granted clients share the OS user and workspace. File roots do not isolate shell commands.")}</p>
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
                    <FormLabel className="text-12">{t("I understand")}{" "}
                      {action === "delete"
                        ? t("the irreversible data loss and cleanup policy")
                        : action === "pause"
                          ? t("that work stops and the workspace is retained")
                          : t("the current policies and shared environment risk")}
                      .
                    </FormLabel>
                  </div>
                  <FormMessage />
                </FormItem>
              )}
            />
            {review.intent && (
              <p className="text-12 text-muted-foreground">{t("A previous request is awaiting a confirmed result. Retrying uses its original body and idempotency key.")}</p>
            )}
            {error && (
              <div role="alert">
                <ErrorBanner message={error} />
              </div>
            )}
            {review.error && (
              <div className="space-y-2 text-12 text-muted-foreground">
                <p>{t("Review the current machine and operation state before discarding an unreadable request. Its previous outcome may be unknown.")}</p>
                <Button
                  type="button"
                  onClick={() => {
                    clearRemoteMachineIntent(ownerId, scope);
                    onReload();
                    onClose();
                  }}
                >{t("Reset unreadable saved action and review")}</Button>
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
              >{t("Reload and review a new action")}</Button>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={onClose}
                disabled={mutation.isPending}
              >{t("Cancel")}</Button>
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
  const { t } = useUiTranslation();
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
    <section aria-label={t("Retained workspace resume preflight")}>
      {quote.error && (
        <p>{t("Resume preflight is unavailable. Submitting repeats current checks; a saved accepted request can still recover its result.")}</p>
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

function RemoteMachineNotifications({
  machineId,
}: {
  readonly machineId: string;
}) {
  const { t } = useUiTranslation();
  const query = useRemoteMachineEvents(machineId);
  const notices = (query.data?.events ?? [])
    .filter(
      (event, index, events) =>
        event.notification &&
        events.findIndex((other) => other.id === event.id) === index,
    )
    .slice(0, 5);
  if (query.error)
    return (
      <p role="status" className="text-12 text-muted-foreground">{t("Machine notifications could not be refreshed. Open the timeline to retry.")}</p>
    );
  if (!notices.length) return null;
  return (
    <section
      aria-label={t("Machine notifications")}
      className="space-y-3 rounded-xl border border-border/50 bg-card p-4"
    >
      <h3 className="text-13 font-semibold">{t("Machine notifications")}</h3>
      {notices.map((event) => (
        <article
          key={event.id}
          className="space-y-1 border-t border-border/30 pt-2"
        >
          <p className="text-12 text-muted-foreground">{event.message}</p>
          <p className="text-11 text-text-tertiary">
            {time(event.created_at)}{t(" · Responsibility: ")}{event.responsibility}
          </p>
          <a
            className="break-all text-11 text-primary-text underline"
            href={`/assistant/machines?machine=${encodeURIComponent(machineId)}&operation=${encodeURIComponent(event.operation_id)}`}
          >{t("View operation ")}{event.operation_id}
          </a>
        </article>
      ))}
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
  const { t } = useUiTranslation();
  const [pages, setPages] = useState<{
    machineId: string;
    cursors: (string | undefined)[];
  }>({ machineId, cursors: [undefined] });
  if (pages.machineId !== machineId)
    setPages({ machineId, cursors: [undefined] });
  const cursor =
    pages.machineId === machineId ? pages.cursors.at(-1) : undefined;
  const query = useRemoteMachineEvents(machineId, cursor);
  const events = query.data?.events ?? [];
  const uniqueEvents = events.filter(
    (event, index) => events.findIndex((row) => row.id === event.id) === index,
  );
  return (
    <div className="space-y-4">
      <p className="text-12 text-muted-foreground">{t("Safe lifecycle events only. This timeline contains no commands, environment values, file contents or job output. Terminal projections remain for 90 days; required active work, cleanup, idempotency and billing evidence are retained separately.")}</p>
      {query.isPending && (
        <p role="status" className="text-12">{t("Loading operation timeline…")}</p>
      )}
      {query.error && (
        <ErrorBanner
          message={t("Timeline unavailable. {{message}}", { message: translateErrorMessage(query.error.message) })}
          onRetry={() => void query.refetch()}
        />
      )}
      {!query.isPending && !query.error && !uniqueEvents.length && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">{t("No lifecycle events are available on this page.")}</p>
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
              <span className="text-11 text-text-tertiary">{t("Responsibility: ")}{event.responsibility}
              </span>
              <Button
                variant="ghost"
                onClick={() => onOperation(event.operation_id)}
              >{t("View operation")}</Button>
            </div>
            <a
              className="break-all text-11 text-primary-text underline"
              href={`/assistant/machines?machine=${encodeURIComponent(machineId)}&operation=${encodeURIComponent(event.operation_id)}`}
            >{t("Operation ")}{event.operation_id}
            </a>
          </li>
        ))}
      </ol>
      <div className="space-y-2 text-12 text-muted-foreground">
        <p>{t("Showing up to 100 events on this page.")}{query.data?.next_cursor === undefined
            ? t(" Older-history availability is unknown on this server.")
            : t(" The server cursor preserves events with matching timestamps.")}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={pages.cursors.length < 2 || query.isFetching}
            onClick={() =>
              setPages({ machineId, cursors: pages.cursors.slice(0, -1) })
            }
          >{t("Newer events")}</Button>
          <Button
            disabled={!query.data?.next_cursor || query.isFetching}
            onClick={() => {
              if (query.data?.next_cursor)
                setPages({
                  machineId,
                  cursors: [...pages.cursors, query.data.next_cursor],
                });
            }}
          >{t("Older events")}</Button>
          <Button
            onClick={() => {
              setPages({ machineId, cursors: [undefined] });
              void query.refetch();
            }}
          >{t("Refresh timeline")}</Button>
        </div>
      </div>
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
  const { t } = useUiTranslation();
  return (
    <div className="space-y-8">
      <DetailSection title={t("Fixed profile and limits")}>
        <DetailRow
          label={t("CPU")}
          value={`${machine.profile.cpu_millis / 1000} vCPU`}
        />
        <DetailRow
          label={t("Memory")}
          value={`${machine.profile.memory_mib / 1024} GiB`}
        />
        <DetailRow
          label={t("Persistent workspace")}
          value={`${machine.profile.workspace_gib} GiB`}
        />
        <DetailRow
          label={t("Temporary storage")}
          value="No fixed capacity declared"
        />
        <DetailRow
          label={t("Concurrent commands")}
          value={String(policy?.concurrent_jobs ?? 2)}
        />
        <DetailRow
          label={t("Command timeout")}
          value={`${policy?.default_command_timeout_seconds ?? 120}s default · ${policy?.maximum_command_timeout_seconds ?? 3600}s maximum`}
        />
        <DetailRow
          label={t("Output limits")}
          value="1 MiB per job (512 KiB each stream) · 32 MiB machine cache"
        />
        <DetailRow
          label={t("Command input")}
          value="32 KiB command · 64 KiB stdin"
        />
        <p className="px-4 py-3 text-12 text-muted-foreground">{t("All subjects and child credentials share two command slots. There is no persistent queue; capacity rejection does not mean work is queued. File reads are paged at up to 4 KiB. Exact edits handle at most 16 MiB for both the original and result; this is not a universal file-size or write-request limit. Smaller gateway and protocol limits still apply. A command can stop before its requested timeout when authentication, grant, execution lease, funding or runtime authority expires.")}</p>
      </DetailSection>
      <DetailSection title={t("Network, environment and sharing")}>
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>{t("Full Machine image includes Chromium, desktop and computer tooling. Only the selected capabilities are enabled; agent grants and live runtime readiness determine use. Only the human owner can watch or take over the desktop. Treat visible browser and desktop content as sensitive. External OAuth and Agent Key tools remain limited to authorized Commands and Files.")}</p>
          <p>{t("This profile has no PTY, inbound ports, public IP or port mapping. It does not offer custom images, upgrades, autoscaling, exports or snapshots. This personal-account profile cannot be transferred to an organization or charged to an organization wallet.")}</p>
          <p>{t("All authorized clients share the Machine and workspace. Commands and file tools use the shared command OS user. Authorized agents also share browser sessions in the existing shared mode. File roots constrain file tools and working directories; they do not isolate shell authority or secrets between clients.")}</p>
          <p>{t("Only /workspace persists; HOME, /tmp, browser profiles and downloads are temporary unless safely saved in the workspace. Temporary storage is managed by the deployment environment; new Machines have no fixed temporary capacity commitment. Temporary data may be lost after pause or Pod recreation. A new runtime does not restore browser sessions or running processes.")}</p>
          <p>{t("There is no persistent environment editor or secret manager. Job environment values are temporary. Downstream credentials stay in NyxID; each job must declare its services and remains bounded by its principal’s live service access and Machine service ceiling.")}</p>
        </div>
      </DetailSection>
      <DetailSection title={t("Retention and charging policy")}>
        <DetailRow label={t("Policy version")} value={machine.policy_version} mono />
        <DetailRow
          label={t("Data policy version")}
          value={machine.data_policy_version}
          mono
        />
        <div className="space-y-3 px-4 py-3 text-12 text-muted-foreground">
          <p>{t("Job metadata remains for ")}{policy?.job_history_days ?? 30}{t(" days; terminal operation projections remain for")}{" "}
            {policy?.operation_history_days ?? 90}{t(" days. Output is read only from the currently authorized live node, retained at most one hour after a job finishes, and can disappear earlier after cache eviction or runtime loss.")}</p>
          <p>{t("Paused workspaces have no automatic expiry and no storage fee. A free machine never implies free downstream services. Runtime and gateway service charges are separate.")}</p>
          <p>{t("Paid runs require an accepted rate and account-balance funding, with no per-instance cap. Account funds and runtime estimates are shared with other Machines and services, and do not promise a duration. Manual resume requires a new accepted quote after earlier liability is closed.")}</p>
        </div>
      </DetailSection>
    </div>
  );
}

function RemoteMachineClientSetup() {
  const { t } = useUiTranslation();
  const config = usePublicConfig();
  const url = config.data?.mcp_url;
  return (
    <DetailSection title={t("Connect existing clients")}>
      <div className="space-y-4 px-4 py-3 text-12 text-muted-foreground">
        <p>{t("Use the same NyxID HTTP MCP endpoint as your existing services. Sign in using the existing OAuth flow, then review the Machine selection. For an Agent Key, manage Machine access on its parent Key detail page; child login credentials inherit that shared grant. No separate principal registration is required.")}</p>
        {!url ? (
          <p role="status">{t("The server MCP URL is unavailable. Open Settings → MCP when configuration is available.")}</p>
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
                    aria-label={t("Copy {{name}} HTTP MCP config", { name: name })}
                    onClick={() =>
                      void copyToClipboard(value).then(
                        () => toast.success(translateUi("{{name}} config copied", { name: name })),
                        () => toast.error(translateUi("Could not copy config")),
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
        <p>{t("Authenticate, discover tools, and use ")}<code>nyx__machine_list</code>{" "}{t("to inspect granted machines before execution. Tool availability still depends on the server’s external authority protocol; configuration alone does not grant access.")}</p>
        <p className="flex gap-2">
          <AlertTriangle className="size-4 shrink-0 text-warning" />{t("Do not paste tokens, credentials or job environment values into these configuration examples or chat.")}</p>
      </div>
    </DetailSection>
  );
}

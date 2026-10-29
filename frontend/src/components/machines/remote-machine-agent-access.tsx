import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useNyxBotAgents,
  useSetNyxBotAgentGrants,
} from "@/hooks/use-nyxbot-agents";
import { useMachineAccess } from "@/hooks/use-machine-access";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { hasPendingMachineApplication } from "@/lib/machine-access";
import { useAuthStore } from "@/stores/auth-store";
import type { MachineAccess } from "@/schemas/machine-access";
import { MachineCapabilityForm } from "@/components/assistant/machine-capabilities";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import type { NodeInfo } from "@/types/nodes";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";

const membershipSchema = z.object({ assigned: z.boolean() });

export function RemoteMachineAgentAccess({
  node,
  machineId,
  initialAgentId,
}: {
  readonly node?: NodeInfo;
  readonly machineId: string;
  readonly initialAgentId?: string;
}) {
  const agents = useNyxBotAgents();
  const userId = useAuthStore((s) => s.user?.id);
  const [agentId, setAgentId] = useState<string | undefined>(initialAgentId);
  const options =
    agents.data?.agents.filter(
      (agent) =>
        agent.status !== "destroyed" &&
        agent.owner_kind !== "org" &&
        (!agent.owner_id || agent.owner_id === userId),
    ) ?? [];
  const selected = options.find((agent) => agent.id === agentId);
  return (
    <div className="space-y-4 px-4 py-3">
      <p className="text-12 text-muted-foreground">
        NyxBot and specialist access uses the existing Agent assignment policy.
        It remains bound to an owner turn; guests cannot use this machine. A
        grant does not bypass a paused runtime or funding requirement.
      </p>
      <p role="status" className="text-12 text-muted-foreground">
        {node?.is_connected === false
          ? "Machine is offline. New execution is unavailable until it reconnects."
          : node?.is_connected === true
            ? "Machine connection is online. Agent grants and runtime admission are checked separately."
            : "Machine connection status is unavailable."}{" "}
        Saved permissions do not confirm runtime availability.
      </p>
      {node ? (
        <OwnerConfirmation node={node} />
      ) : (
        <p className="text-12 text-muted-foreground">
          Confirmation settings will be available when the Node record can be
          loaded.
        </p>
      )}
      {agents.error ? (
        <ErrorBanner
          message="Agent assignments could not be loaded."
          onRetry={() => void agents.refetch()}
        />
      ) : agents.isPending ? (
        <p role="status" className="text-12">
          Loading agents…
        </p>
      ) : (
        <>
          <label className="block space-y-2 text-12">
            <span>Agent</span>
            <Select value={selected?.id ?? ""} onValueChange={setAgentId}>
              <SelectTrigger aria-label="Agent for machine access">
                <SelectValue placeholder="Choose an existing agent" />
              </SelectTrigger>
              <SelectContent>
                {options.map((agent) => (
                  <SelectItem key={agent.id} value={agent.id}>
                    {agent.display_name ?? agent.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          {!options.length ? (
            <p className="text-12 text-muted-foreground">
              No eligible personal agents are available.
            </p>
          ) : null}
          {selected ? (
            <AgentAssignment
              key={`${userId}:${machineId}:${selected.id}`}
              machineId={machineId}
              agent={selected}
            />
          ) : null}
        </>
      )}
    </div>
  );
}

function OwnerConfirmation({ node }: { readonly node: NodeInfo }) {
  return (
    <section
      aria-label="Remote machine owner confirmation"
      className="space-y-3 rounded-lg border border-border p-3"
    >
      <div className="flex flex-wrap items-center gap-2 text-12">
        <span className="font-medium">Owner confirmation policy</span>
        <span className="rounded-md border border-border bg-muted px-2 py-0.5">
          {node.machine_confirm === "changes"
            ? "Confirm changes"
            : node.machine_confirm === "all"
              ? "Confirm every operation"
              : node.machine_confirm === "none"
                ? "No machine-specific confirmation"
                : "Not reported"}
        </span>
      </div>
      <p className="text-12 text-muted-foreground">
        Read from the current Node record. This hosted machine detail view does
        not save confirmation settings; applicable Assistant confirmation rules
        still apply to each operation.
      </p>
    </section>
  );
}

function AgentAssignment({
  agent,
  machineId,
}: {
  readonly agent: AssistantAgent;
  readonly machineId: string;
}) {
  const mutation = useSetNyxBotAgentGrants();
  const query = useMachineAccess(agent.id);
  const assigned = agent.machines?.includes(machineId) ?? false;
  const form = useAppForm<z.infer<typeof membershipSchema>>({
    resolver: zodResolver(membershipSchema),
    defaultValues: { assigned },
    values: { assigned },
  });
  const [error, setError] = useState<string>();
  const [savedAssignment, setSavedAssignment] = useState<boolean>();
  const membershipSaved =
    savedAssignment !== undefined && savedAssignment === assigned;
  const [capabilitySaveFailed, setCapabilitySaveFailed] = useState(false);
  const assignment = query.data?.find((row) => row.node_id === machineId);
  return (
    <div className="space-y-3">
      {agent.kind === "nyxbot" ? (
        <p className="text-12 text-muted-foreground">
          NyxBot has owner machine membership. Its capability policy and the
          live owner-turn gate still apply.
        </p>
      ) : (
        <form
          className="space-y-3"
          aria-label="Specialist machine membership"
          onSubmit={form.handleSubmit(async (data) => {
            if (
              mutation.isPending ||
              agent.can_maintain === false ||
              !form.formState.isDirty
            )
              return;
            setError(undefined);
            setCapabilitySaveFailed(false);
            try {
              const grants = {
                id: agent.id,
                services: agent.services,
                account_read: agent.account_read,
                machines: data.assigned
                  ? [...new Set([...(agent.machines ?? []), machineId])]
                  : (agent.machines ?? []).filter((id) => id !== machineId),
                ...(agent.logins !== undefined ? { logins: agent.logins } : {}),
                ...(agent.guest_access
                  ? { guest_access: agent.guest_access }
                  : {}),
              };
              await mutation.mutateAsync(grants);
              form.reset(data);
              setSavedAssignment(data.assigned);
              const refreshed = await query.refetch();
              if (refreshed.error) {
                setError(
                  "Assignment was saved, but the current capability state could not be refreshed.",
                );
              }
            } catch (cause) {
              setError(
                cause instanceof Error
                  ? cause.message
                  : "Could not save assignment.",
              );
            }
          })}
        >
          <label className="flex items-start gap-2 text-12">
            <Checkbox
              disabled={mutation.isPending || agent.can_maintain === false}
              checked={form.watch("assigned")}
              onCheckedChange={(value) =>
                form.setValue("assigned", value === true)
              }
            />
            Assign this machine to {agent.display_name ?? agent.name}
          </label>
          <p className="text-12 text-muted-foreground">
            Hosted assignments create membership only. New assignments start
            with all tools and saved logins off, regardless of editing flags.
            Review the current policy after assigning. Removing membership
            requests cancellation of this agent’s work on this machine; delivery
            and cleanup still need confirmation.
          </p>
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          ) : null}
          {membershipSaved && !error ? (
            <p role="status" className="text-12 text-success">
              Machine assignment saved. Review the capability state below; a
              capability save is a separate operation and does not roll back
              this assignment.
            </p>
          ) : null}
          {membershipSaved && capabilitySaveFailed ? (
            <p role="alert" className="text-12 text-warning">
              Assignment succeeded, but capability changes failed. The
              assignment remains in place; reload the current capability state
              and retry editing when it is available.
            </p>
          ) : null}
          <Button
            type="submit"
            disabled={
              mutation.isPending ||
              !form.formState.isDirty ||
              agent.can_maintain === false
            }
            isLoading={mutation.isPending}
          >
            Save assignment
          </Button>
        </form>
      )}
      {query.error ? (
        <ErrorBanner
          message="Could not load the current capability revision."
          onRetry={() => void query.refetch()}
        />
      ) : assignment ? (
        <HostedCapabilities
          key={agent.id}
          agentId={agent.id}
          machine={assignment}
          membership={
            agent.kind === "nyxbot"
              ? "owner"
              : assigned
                ? "assigned"
                : "not_assigned"
          }
          disabled={agent.can_maintain === false}
          onSaveError={() => {
            setCapabilitySaveFailed(true);
          }}
          onSaveSuccess={() => {
            setCapabilitySaveFailed(false);
          }}
        />
      ) : (
        <p className="text-12 text-muted-foreground">
          Capabilities are not yet available from this Node.
        </p>
      )}
      <Button
        type="button"
        size="sm"
        variant="secondary"
        disabled={query.isFetching}
        onClick={() => void query.refetch()}
      >
        Refresh machine status
      </Button>
      {query.data?.some(hasPendingMachineApplication) ? (
        <p className="text-12 text-muted-foreground">
          Status refreshes for up to one minute while this page is active. Use
          Refresh machine status for a later check.
        </p>
      ) : null}
    </div>
  );
}

function HostedCapabilities({
  agentId,
  machine,
  membership,
  disabled,
  onSaveError,
  onSaveSuccess,
  availableLogins,
  sharedLoginsAllowed,
}: {
  readonly agentId: string;
  readonly machine: MachineAccess;
  readonly membership: "owner" | "assigned" | "not_assigned";
  readonly disabled: boolean;
  readonly onSaveError?: () => void;
  readonly onSaveSuccess?: () => void;
  readonly availableLogins?: readonly {
    id: string;
    label: string;
    username_hint: string;
    eligible: boolean;
  }[];
  readonly sharedLoginsAllowed?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  const contextsEnabled = useFeature(FEATURE_FLAG.MACHINE_CONTEXTS);
  return (
    <section aria-label="Hosted agent capabilities" className="space-y-3">
      <p className="text-12">Policy revision {machine.revision}</p>
      <p className="text-12 text-muted-foreground">
        Saving requests cancellation of this agent’s current work on this
        machine. Browser is required for Computer and Developer browser. Turning
        off Browser also removes Computer and Developer browser; other grants
        are preserved. External OAuth and Agent Key clients remain restricted to
        their existing shell/file tools and do not gain Browser or Computer
        access here.
      </p>
      <MachineCapabilityForm
        machine={machine}
        agentId={agentId}
        membership={membership}
        disabled={disabled}
        capabilityEditing={enabled}
        contextEnabled={contextsEnabled}
        onSaveError={onSaveError}
        onSaveSuccess={onSaveSuccess}
        availableLogins={availableLogins}
        sharedLoginsAllowed={sharedLoginsAllowed}
        configurationApplicability="hosted"
      />
    </section>
  );
}

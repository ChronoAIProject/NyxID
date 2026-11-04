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
import { useAuthStore } from "@/stores/auth-store";
import type { MachineAccess } from "@/schemas/machine-access";
import { MachineCapabilityForm } from "@/components/assistant/machine-capabilities";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import type { NodeInfo } from "@/types/nodes";
import { useAppForm } from "@/components/ui/form";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedFormScope,
  GroupedFormActions,
  GroupedFormBar,
} from "@/components/ui/grouped-form";
import {
  GroupedDraftOwnerContext,
  useGroupedDraftGuard,
} from "@/lib/grouped-drafts";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";

const membershipSchema = z.object({ assigned: z.boolean() });

export function RemoteMachineAgentAccess({
  node,
  machineId,
}: {
  readonly node?: NodeInfo;
  readonly machineId: string;
}) {
  const agents = useNyxBotAgents();
  const userId = useAuthStore((s) => s.user?.id);
  const [agentId, setAgentId] = useState<string>();
  const guard = useGroupedDraftGuard();
  const options =
    agents.data?.agents.filter(
      (agent) =>
        agent.status !== "destroyed" &&
        agent.owner_kind !== "org" &&
        (!agent.owner_id || agent.owner_id === userId),
    ) ?? [];
  const selected = options.find((agent) => agent.id === agentId);
  return (
    <GroupedDraftOwnerContext value="machine-access">
      <div className="space-y-6 px-4 py-3">
        {node ? (
          <OwnerConfirmation node={node} />
        ) : (
          <GroupedList
            title="Owner confirmation"
            footer="Confirmation settings will be available when the Node record can be loaded."
          >
            <GroupedRow title="Node record unavailable" />
          </GroupedList>
        )}
        {agents.error ? (
          <GroupedList footer={<ErrorBanner
            message="Agent assignments could not be loaded."
            onRetry={() => void agents.refetch()}
          />} />
        ) : agents.isPending ? (
          <GroupedList><GroupedRow role="status" title="Loading agents…" /></GroupedList>
        ) : (
          <>
            <GroupedList
              title="Agent access"
              footer={
                <>
                  NyxBot and specialist access uses the existing Agent
                  assignment policy. It remains bound to an owner turn; guests
                  cannot use this machine. A grant does not bypass a paused
                  runtime or funding requirement.
                  {!options.length && (
                    <p>No eligible personal agents are available.</p>
                  )}
                </>
              }
            >
              <GroupedRow
                title="Agent"
                accessory={
                  <Select
                    value={selected?.id ?? ""}
                    onValueChange={async (id) => {
                      if (
                        !guard ||
                        (await guard.confirmDiscard("machine-access"))
                      )
                        setAgentId(id);
                    }}
                  >
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
                }
              />
            </GroupedList>
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
    </GroupedDraftOwnerContext>
  );
}

function OwnerConfirmation({ node }: { readonly node: NodeInfo }) {
  return (
    <GroupedList
      title="Owner confirmation"
      aria-label="Remote machine owner confirmation"
      footer="Read from the current Node record. This hosted machine detail view does not save confirmation settings; applicable Assistant confirmation rules still apply to each operation."
    >
      <GroupedRow
        title="Owner confirmation policy"
        accessory={
          node.machine_confirm === "changes"
            ? "Confirm changes"
            : node.machine_confirm === "all"
              ? "Confirm every operation"
              : node.machine_confirm === "none"
                ? "No machine-specific confirmation"
                : "Not reported"
        }
      />
    </GroupedList>
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
  const save = form.handleSubmit(async (data) => {
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
  });
  return (
    <GroupedFormScope>
      <div className="space-y-6">
        {agent.kind === "nyxbot" ? (
          <GroupedList
            title="Machine membership"
            footer="NyxBot has owner machine membership. Its capability policy and the live owner-turn gate still apply."
          >
            <GroupedRow title="Owner machine membership" />
          </GroupedList>
        ) : (
          <form
            className="space-y-3"
            aria-label="Specialist machine membership"
            onSubmit={save}
          >
            <GroupedList
              title="Machine membership"
              footer={
                <>
                  <p>
                    With capability editing enabled, new assignments start with
                    capabilities off. Review the current policy after assigning.
                    Removing membership cancels this agent’s work on this
                    machine.
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
                </>
              }
            >
              <GroupedRow asChild interactive>
                <label>
                  {" "}
                  <Checkbox
                    checked={form.watch("assigned")}
                    onCheckedChange={(value) =>
                      form.setValue("assigned", value === true)
                    }
                  />
                  Assign this machine to {agent.display_name ?? agent.name}
                </label>
              </GroupedRow>
            </GroupedList>
            <GroupedFormActions
              label="machine membership"
              dirty={form.formState.isDirty}
              pending={mutation.isPending}
              disabled={agent.can_maintain === false}
              save={save}
              discard={() => {
                form.reset();
                setError(undefined);
              }}
              saveLabel="Save assignment"
            />
          </form>
        )}
        {query.error ? (
          <ErrorBanner
            message="Could not load the current capability revision."
            onRetry={() => void query.refetch()}
          />
        ) : assignment ? (
          <HostedCapabilities
            key={`${agent.id}:${assignment.revision}`}
            agentId={agent.id}
            machine={assignment}
            onSaveError={() => {
              setCapabilitySaveFailed(true);
            }}
            onSaveSuccess={() => {
              setCapabilitySaveFailed(false);
            }}
          />
        ) : (
          <GroupedList title="Capabilities">
            <GroupedRow title="Capabilities are not yet available from this Node." />
          </GroupedList>
        )}
        <GroupedFormBar />
      </div>
    </GroupedFormScope>
  );
}

function HostedCapabilities({
  agentId,
  machine,
  onSaveError,
  onSaveSuccess,
}: {
  readonly agentId: string;
  readonly machine: MachineAccess;
  readonly onSaveError?: () => void;
  readonly onSaveSuccess?: () => void;
}) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  return (
    <section aria-label="Hosted agent capabilities" className="space-y-3">
      <GroupedList
        title="Capability policy"
        footer={
          <p>
            Saving cancels this agent’s current work on this machine. Browser is
            required for Computer and Developer browser. Turning off Browser
            also removes Computer and Developer browser; other grants are
            preserved. External OAuth and Agent Key clients remain restricted to
            their existing shell/file tools and do not gain Browser or Computer
            access here.
          </p>
        }
      >
        <GroupedRow
          title="Policy revision"
          accessory={String(machine.revision)}
        />
      </GroupedList>
      {!enabled || !machine.protocol_v2 ? (
        <GroupedList footer="Capability editing is unavailable until the account feature and Node protocol support are enabled.">
          <GroupedRow role="status" title="Capability editing unavailable" />
        </GroupedList>
      ) : null}
      <MachineCapabilityForm
        machine={machine}
        agentId={agentId}
        disabled={!enabled}
        onSaveError={onSaveError}
        onSaveSuccess={onSaveSuccess}
      />
      {machine.revocation_pending ? (
        <GroupedList footer="Revocation delivery is pending; this is not proof of physical runtime stop.">
          <GroupedRow role="status" title="Revocation pending" />
        </GroupedList>
      ) : null}
    </section>
  );
}

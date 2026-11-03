import type { ContentScreenProps } from "../../app/navigationTypes";
import { AgentAvatar } from "../../components/AgentAvatar";
import { useRef, useState, type PropsWithChildren } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import {
  useOpenWorkspaceChat,
  useWorkspaceBack,
} from "../../app/WorkspaceHeader";
import {
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { agentMachinesApi } from "../../lib/api/agentMachinesApi";
import { machinePageContent, MachineSwitch } from "./MachinePresentation";
import {
  confirmDiscard,
  machineError,
  machineKeys,
  useMachineRuntime,
  useMachineWrite,
} from "./MachineUI";
import {
  capabilityLabels,
  machineAccessState,
  selectionRequired,
  separationAvailable,
  setMachineCapability,
  SHARED_LEGACY_WARNING,
  type AgentMachine,
  type ContextMode,
  type MachineCapabilities,
} from "./agentMachineContracts";

export function agentMachineModeLabel(machine: AgentMachine) {
  return selectionRequired(machine)
    ? "Owner selection required"
    : machine.mode === "separated"
      ? "Separate workspace and browser"
      : "Shared legacy";
}

export function AgentMachinesScreen({
  route,
  navigation,
}:
  | ContentScreenProps<"AgentMachines">
  | ContentScreenProps<"AgentMachineEditor">) {
  const runtime = useMachineRuntime();
  const client = useQueryClient();
  const openChat = useOpenWorkspaceChat();
  const selected =
    route.name === "AgentMachineEditor" ? route.params.machineId : undefined;
  const [notice, setNotice] = useState<string>();
  const refreshedAt = useRef(0);
  const queryKey = [
    ...machineKeys(runtime.epoch),
    "agent",
    route.params.agentId,
  ];
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) =>
      agentMachinesApi.read(route.params.agentId, signal),
    enabled: runtime.active,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const machine = query.data?.machines.find((row) => row.node_id === selected);
  async function refresh() {
    if (Date.now() - refreshedAt.current < 1000)
      throw new Error("Wait a moment before refreshing again.");
    refreshedAt.current = Date.now();
    const result = await query.refetch();
    if (result.error) throw result.error;
    return result.data?.machines.find((row) => row.node_id === selected);
  }
  if (machine && query.data) {
    return (
      <AgentMachineEditor
        key={`${runtime.epoch}:${route.params.agentId}:${machine.node_id}:${machine.revision}`}
        machine={machine}
        notice={notice}
        readError={query.error ? machineError(query.error) : undefined}
        agentId={route.params.agentId}
        ownerId={query.data.agent.owner_id}
        agentName={query.data.agent.display_name ?? query.data.agent.name}
        features={query.data.features}
        unavailable={!!query.error}
        onBack={navigation.goBack}
        onRefresh={refresh}
        onSaved={(machines) => {
          client.setQueryData(queryKey, { ...query.data, machines });
          setNotice(
            "Capabilities saved. Review cleanup and acknowledgement status before starting new work.",
          );
          void client.invalidateQueries({
            queryKey: ["workspace", "agents"],
          });
        }}
        onOpenApproval={(conversationId, acknowledgementId) =>
          openChat({ conversationId, acknowledgementId })
        }
      />
    );
  }
  return (
    <AgentMachineListPage
      selected={!!selected}
      onBack={navigation.goBack}
      refreshing={query.isRefetching}
      onRefresh={() => {
        if (runtime.active) {
          void refresh().catch((cause) => {
            if (runtime.current()) setNotice(machineError(cause));
          });
        }
      }}
    >
      {notice && <Notice message={notice} />}
      {query.isPending && <Label muted>Loading agent machine access…</Label>}
      {query.error && (
        <Notice tone="error" message={machineError(query.error)} />
      )}
      {query.data && !selected && !query.error && (
        <>
          <SettingsGroup
            inset={false}
            footer="Machine permissions only narrow the local capability ceiling. Saving cancels this agent's current work on that machine. External API-key and OAuth grants remain on the web."
          >
            <SettingsRow
              leading={<AgentAvatar agent={query.data.agent} size="md" />}
              label={query.data.agent.display_name ?? query.data.agent.name}
            />
          </SettingsGroup>
          <SettingsGroup title="Machines" inset={false}>
            {query.data.machines.map((row) => (
              <SettingsRow
                key={row.node_id}
                label={row.name}
                detail={machineAccessState(row).join(" ") || undefined}
                value={agentMachineModeLabel(row)}
                valueTone={
                  selectionRequired(row) || row.mode !== "separated"
                    ? "danger"
                    : "default"
                }
                accessibilityLabel={`Review ${row.name} access. ${[
                  agentMachineModeLabel(row),
                  `Revision ${row.revision}`,
                  ...machineAccessState(row),
                ].join(". ")}`}
                onPress={() =>
                  navigation.push("AgentMachineEditor", {
                    agentId: route.params.agentId,
                    machineId: row.node_id,
                  })
                }
              />
            ))}
            {!query.data.machines.length && (
              <SettingsRow label="No machines are available to this agent." />
            )}
          </SettingsGroup>
        </>
      )}
      {selected && query.data && !machine && (
        <Notice message="This machine is no longer available to this agent. Return to the list and review current access." />
      )}
    </AgentMachineListPage>
  );
}

function AgentMachineListPage({
  selected,
  onBack,
  refreshing,
  onRefresh,
  children,
}: PropsWithChildren<{
  selected: boolean;
  onBack: () => void;
  refreshing: boolean;
  onRefresh: () => void;
}>) {
  useWorkspaceBack({ active: false, defaultRole: "back", onBack });
  return (
    <WorkspacePage
      title="Agent machine access"
      showHeading={false}
      refreshing={refreshing}
      onRefresh={onRefresh}
      contentStyle={machinePageContent}
    >
      {children}
    </WorkspacePage>
  );
}

export function AgentMachineEditor({
  machine,
  agentId,
  ownerId,
  agentName,
  features,
  unavailable,
  notice,
  readError,
  onBack,
  onRefresh,
  onSaved,
  onOpenApproval,
}: {
  machine: AgentMachine;
  agentId: string;
  ownerId?: string;
  agentName: string;
  features: string[];
  unavailable: boolean;
  notice?: string;
  readError?: string;
  onBack: () => void;
  onRefresh: () => Promise<AgentMachine | undefined>;
  onSaved: (machines: AgentMachine[]) => void;
  onOpenApproval: (conversationId: string, acknowledgementId: string) => void;
}) {
  const write = useMachineWrite();
  const [caps, setCaps] = useState(machine.capabilities);
  const [legacyAccepted, setLegacyAccepted] = useState(false);
  const [confirmMode, setConfirmMode] = useState<ContextMode>();
  const [approval, setApproval] = useState<{
    conversation_id: string;
    acknowledgement_id: string;
  }>();
  const [refreshError, setRefreshError] = useState<string>();
  const [refreshing, setRefreshing] = useState(false);
  const dirty = JSON.stringify(caps) !== JSON.stringify(machine.capabilities);
  const blocked = write.busy || write.uncertain || !write.active || unavailable;
  const editable =
    features.includes("assistant:machine-capabilities") &&
    machine.can_edit &&
    machine.protocol_v2 &&
    !selectionRequired(machine);
  useWorkspaceBack({
    active: Boolean(confirmMode),
    defaultRole: "back",
    disabled: write.busy,
    onBack: () => (confirmMode ? setConfirmMode(undefined) : onBack()),
    discardOnClose:
      dirty || confirmMode
        ? {
            title: "Discard machine access changes?",
            message:
              "Unsaved capability changes and context review will be closed.",
          }
        : undefined,
  });
  async function refresh() {
    if (write.busy || !write.active) return;
    setRefreshing(true);
    try {
      const latest = await onRefresh();
      if (!write.current()) return;
      if (!latest)
        throw new Error("This machine is no longer available to this agent.");
      setCaps(latest.capabilities);
      setConfirmMode(undefined);
      setLegacyAccepted(false);
      write.reviewed();
      setRefreshError(undefined);
    } catch (cause) {
      if (write.current()) setRefreshError(machineError(cause));
    } finally {
      setRefreshing(false);
    }
  }
  function reviewRefresh() {
    if (!write.busy && write.active && !refreshing) {
      confirmDiscard(dirty, () => void refresh());
    }
  }
  return (
    <WorkspacePage
      title="Agent machine access"
      showHeading={false}
      refreshing={refreshing}
      onRefresh={reviewRefresh}
      contentStyle={machinePageContent}
    >
      {notice && <Notice message={notice} />}
      {readError && <Notice tone="error" message={readError} />}
      <SettingsGroup
        inset={false}
        footer={
          [machine.execution_note, machine.separated_setup_note]
            .filter(Boolean)
            .join("\n\n") || undefined
        }
      >
        <SettingsRow label="Agent" value={agentName} />
        <SettingsRow label="Machine" value={machine.name} />
        <SettingsRow label="Revision" value={String(machine.revision)} />
        <SettingsRow
          label="Mode"
          value={agentMachineModeLabel(machine)}
          valueTone={
            machine.mode === "separated" && !selectionRequired(machine)
              ? "default"
              : "danger"
          }
        />
        <SettingsRow
          label="Separation"
          value={
            separationAvailable(machine)
              ? `Available (Landlock ABI ${machine.separated?.landlock_abi ?? "unknown"})`
              : (machine.separated?.reason ?? "Unavailable")
          }
        />
      </SettingsGroup>
      {machineAccessState(machine).map((message) => (
        <Notice key={message} message={message} />
      ))}
      {write.error && <Notice tone="error" message={write.error} />}
      {refreshError && <Notice tone="error" message={refreshError} />}
      {!editable && (
        <Notice
          message={
            !features.includes("assistant:machine-capabilities")
              ? "Capability editing is not enabled. Existing restrictions remain in effect."
              : !machine.can_edit
                ? "Only the machine owner or an organization Admin can edit access."
                : !machine.protocol_v2
                  ? "Update this machine before editing capabilities. Existing access continues."
                  : "Complete owner selection before editing capabilities."
          }
        />
      )}
      <SettingsGroup
        title="Capabilities"
        inset={false}
        footer={`Shell uses its OS user's full permissions even with File tools off. Computer control operates applications. Developer browser permits JavaScript in its separate browser; saved logins stay in Secure browser.\n\nSaved-login access is preserved: ${
          machine.saved_login_ids === null
            ? "legacy selection"
            : `${machine.saved_login_ids.length} selected`
        }. Review saved-login grants on the web.`}
      >
        {(Object.keys(capabilityLabels) as (keyof MachineCapabilities)[]).map(
          (key) => (
            <MachineSwitch
              key={key}
              label={capabilityLabels[key]}
              detail={machine.ceiling[key] ? undefined : "Off on machine"}
              accessibilityLabel={`${capabilityLabels[key]}${machine.ceiling[key] ? "" : " (off on machine)"}`}
              value={caps[key]}
              disabled={
                blocked ||
                !editable ||
                !machine.ceiling[key] ||
                ((key === "computer" || key === "developer_browser") &&
                  !caps.browser)
              }
              onChange={(value) =>
                setCaps((current) => setMachineCapability(current, key, value))
              }
            />
          ),
        )}
      </SettingsGroup>
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label={write.busy ? "Saving…" : "Save Capabilities"}
          accessibilityLabel="Save agent machine capabilities"
          disabled={blocked || !editable || !dirty}
          onPress={() =>
            void write.run(async (current) => {
              const result = await agentMachinesApi.save(
                agentId,
                ownerId,
                machine,
                caps,
                current,
              );
              if (current()) onSaved(result);
            })
          }
        />
      </SettingsGroup>
      {machine.can_edit && (
        <>
          {machine.mode !== "shared_legacy" && (
            <Notice message={SHARED_LEGACY_WARNING} />
          )}
          <SettingsGroup
            title="Workspace and browser mode"
            inset={false}
            footer="Mode changes require a separate owner action card in the Assistant. Requesting approval does not apply the change. Save or discard capability edits first."
          >
            <SettingsRow
              label="Current mode"
              value={agentMachineModeLabel(machine)}
            />
            {features.includes("assistant:machine-contexts") &&
              machine.mode !== "separated" &&
              separationAvailable(machine) && (
                <SettingsRow
                  tone="action"
                  label="Review Separate Workspace Request"
                  accessibilityLabel="Review separate workspace request"
                  disabled={blocked || dirty || !!approval}
                  onPress={() => setConfirmMode("separated")}
                />
              )}
            {machine.mode !== "shared_legacy" && (
              <MachineSwitch
                label="Choose shared legacy access"
                detail="I acknowledge the risk"
                accessibilityLabel="I explicitly choose shared legacy access and acknowledge the risk"
                value={legacyAccepted}
                disabled={blocked || dirty || !!approval}
                onChange={setLegacyAccepted}
              />
            )}
            {machine.mode !== "shared_legacy" && (
              <SettingsRow
                tone="action"
                label="Review Shared Legacy Request"
                accessibilityLabel="Review shared legacy request"
                disabled={blocked || dirty || !legacyAccepted || !!approval}
                onPress={() => setConfirmMode("shared_legacy")}
              />
            )}
          </SettingsGroup>
          {confirmMode && (
            <>
              <Notice
                message={`Request owner approval for ${agentName} on ${machine.name}: ${confirmMode === "separated" ? "separate workspace and browser" : "shared legacy access"}? Current capability and saved-login selections are preserved.`}
              />
              <SettingsGroup inset={false}>
                <SettingsRow
                  tone="action"
                  label="Request Owner Action Card"
                  accessibilityLabel="Request owner action card"
                  disabled={
                    blocked ||
                    dirty ||
                    (confirmMode === "shared_legacy" && !legacyAccepted)
                  }
                  onPress={() =>
                    void write.run(async (current) => {
                      const result = await agentMachinesApi.requestContext(
                        agentId,
                        ownerId,
                        machine,
                        confirmMode,
                        current,
                      );
                      if (!current()) return;
                      setApproval(result);
                      setConfirmMode(undefined);
                    })
                  }
                />
                <SettingsRow
                  tone="action"
                  label="Cancel Context Request"
                  accessibilityLabel="Cancel context request"
                  disabled={write.busy}
                  onPress={() => setConfirmMode(undefined)}
                />
              </SettingsGroup>
            </>
          )}
          {approval && (
            <>
              <Notice message="Owner approval requested. Review the action card in its Assistant conversation. Access changes only after approval succeeds." />
              <SettingsGroup inset={false}>
                <SettingsRow
                  label="Review Owner Action Card in Assistant"
                  accessibilityLabel="Review owner action card in Assistant"
                  disabled={write.busy}
                  onPress={() =>
                    onOpenApproval(
                      approval.conversation_id,
                      approval.acknowledgement_id,
                    )
                  }
                />
              </SettingsGroup>
            </>
          )}
        </>
      )}
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label="Reload Access and Recovery Status"
          accessibilityLabel="Reload access and recovery status"
          disabled={write.busy || !write.active || refreshing}
          onPress={reviewRefresh}
        />
      </SettingsGroup>
    </WorkspacePage>
  );
}

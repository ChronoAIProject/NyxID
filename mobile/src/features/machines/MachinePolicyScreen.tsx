import { useState } from "react";
import { Switch, View } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type { WorkspaceContentParamList } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { machinesApi } from "../../lib/api/machinesApi";
import {
  confirmDiscard,
  machineError,
  machineKeys,
  useMachineRuntime,
  useMachineWrite,
} from "./MachineUI";
import {
  updateInProgress,
  updateResultMessage,
  type MachineNode,
  type MachineUpdate,
} from "./machineContracts";

export function MachineSwitch({
  label,
  value,
  disabled,
  onChange,
}: {
  label: string;
  value: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <View
      style={{
        flexDirection: "row",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 12,
        minHeight: 44,
      }}
    >
      <Label style={{ flexGrow: 1, flexShrink: 1 }}>{label}</Label>
      <Switch
        accessibilityLabel={label}
        value={value}
        disabled={disabled}
        onValueChange={onChange}
        style={{ minHeight: 44 }}
      />
    </View>
  );
}

export function MachinePolicyScreen({
  navigation,
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "MachinePolicy">) {
  const runtime = useMachineRuntime();
  const query = useQuery({
    queryKey: [...machineKeys(runtime.epoch), "policy", route.params.machineId],
    queryFn: ({ signal }) =>
      machinesApi.authorizedNode(
        route.params.machineId,
        route.params.ownerId,
        signal,
      ),
    enabled: runtime.active,
    retry: false,
  });
  return (
    <WorkspacePage title="Owner policy and updates" showHeading={false}>
      {query.isPending && <Label muted>Loading machine authority…</Label>}
      {query.error && (
        <>
          <Notice tone="error" message={machineError(query.error)} />
          <ActionButton
            label="Refresh machine authority"
            variant="secondary"
            onPress={() => void query.refetch()}
          />
        </>
      )}
      {query.data && !query.error && (
        <PolicyEditor
          key={`${runtime.epoch}:${route.params.machineId}`}
          node={query.data.node}
          update={query.data.inventory.updates.find(
            (row) => row.node_id === route.params.machineId,
          )}
          onBack={navigation.goBack}
          onRefresh={async () => {
            const result = await query.refetch();
            if (result.error) throw result.error;
            if (!result.data) throw new Error("Machine unavailable");
            return result.data.node;
          }}
        />
      )}
    </WorkspacePage>
  );
}

export function PolicyEditor({
  node,
  update,
  onBack,
  onRefresh,
}: {
  node: MachineNode;
  update?: MachineUpdate;
  onBack: () => void;
  onRefresh: () => Promise<MachineNode>;
}) {
  const write = useMachineWrite();
  const client = useQueryClient();
  const [baseline, setBaseline] = useState(node);
  const [confirm, setConfirm] = useState(node.machine_confirm);
  const [allow, setAllow] = useState(node.allow_single_user_saved_logins);
  const [ack, setAck] = useState(false);
  const [automaticDraft, setAutomaticDraft] = useState<boolean>();
  const [confirmUpdate, setConfirmUpdate] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [refreshError, setRefreshError] = useState<string>();
  const automatic = automaticDraft ?? update?.automatic ?? false;
  const policyDirty =
    confirm !== baseline.machine_confirm ||
    allow !== baseline.allow_single_user_saved_logins;
  const automaticDirty = !!update && automatic !== update.automatic;
  const dirty = policyDirty || automaticDirty;
  const blocked = write.busy || write.uncertain || !write.active;
  const needsAck =
    allow &&
    !baseline.allow_single_user_saved_logins &&
    !node.machine?.browser_isolated;
  useWorkspaceBack({
    active: confirmUpdate,
    defaultRole: "back",
    disabled: write.busy,
    onBack: () => (confirmUpdate ? setConfirmUpdate(false) : onBack()),
    discardOnClose:
      dirty || confirmUpdate
        ? {
            title: "Discard machine changes?",
            message:
              "Unsaved policy changes and pending update confirmation will be closed.",
          }
        : undefined,
  });
  async function refresh() {
    if (write.busy) return;
    try {
      const latest = await onRefresh();
      if (!write.current()) return;
      setBaseline(latest);
      setConfirm(latest.machine_confirm);
      setAllow(latest.allow_single_user_saved_logins);
      setAck(false);
      setAutomaticDraft(undefined);
      write.reviewed();
      setRefreshError(undefined);
    } catch (cause) {
      if (write.current()) setRefreshError(machineError(cause));
    }
  }
  async function settled() {
    await client.invalidateQueries({ queryKey: machineKeys(write.epoch) });
  }
  return (
    <>
      <Card>
        <Label>
          {node.name} · {node.owner.display_name}
        </Label>
        <Label muted>
          Owner policy does not change the local capability ceiling or any
          agent's grant.
        </Label>
      </Card>
      {write.error && <Notice tone="error" message={write.error} />}
      {refreshError && <Notice tone="error" message={refreshError} />}
      {notice && <Notice message={notice} />}
      <ActionButton
        label="Reload policy and update status"
        variant="secondary"
        disabled={write.busy || !write.active}
        onPress={() => confirmDiscard(dirty, () => void refresh())}
      />
      <Card>
        <Label>Machine confirmation</Label>
        {(
          [
            ["none", "No machine confirmation"],
            ["changes", "Confirm changes"],
            ["all", "Confirm every operation"],
          ] as const
        ).map(([value, label]) => (
          <ActionButton
            key={value}
            label={`${confirm === value ? "Selected: " : ""}${label}`}
            variant="secondary"
            disabled={blocked}
            onPress={() => setConfirm(value)}
          />
        ))}
        <MachineSwitch
          label="Allow saved-login typing on a single-user browser"
          value={allow}
          disabled={blocked}
          onChange={(value) => {
            setAllow(value);
            setAck(false);
          }}
        />
        {needsAck && (
          <>
            <Notice message="Commands run as the browser user. A misbehaving or prompt-injected agent could read typed values. Prefer a machine container or a VM with separate browser and agent users." />
            <MachineSwitch
              label="I acknowledge the risk of typed saved-login values being read"
              value={ack}
              disabled={blocked}
              onChange={setAck}
            />
          </>
        )}
        <ActionButton
          label="Save owner policy"
          loading={write.busy}
          disabled={blocked || !policyDirty || (needsAck && !ack)}
          onPress={() =>
            void write.run(async (current) => {
              await machinesApi.policy(
                baseline,
                {
                  machine_confirm: confirm,
                  allow_single_user_saved_logins: allow,
                  acknowledge_single_user_risk: ack,
                },
                current,
              );
              if (!current()) return;
              setBaseline({
                ...baseline,
                machine_confirm: confirm,
                allow_single_user_saved_logins: allow,
              });
              setAck(false);
              setNotice(
                "Owner policy saved. Automatic-update policy is saved separately.",
              );
              await settled();
            })
          }
        />
      </Card>
      {update ? (
        <Card>
          <Label>
            Updates: {update.current_version} → {update.target_version}
          </Label>
          <Label muted>
            Phase: {update.phase} {update.code ?? ""}
          </Label>
          {update.guidance && <Label muted>{update.guidance}</Label>}
          {update.updater_guidance && (
            <Label muted>{update.updater_guidance}</Label>
          )}
          <MachineSwitch
            label="Automatic machine updates"
            value={automatic}
            disabled={blocked}
            onChange={setAutomaticDraft}
          />
          <ActionButton
            label="Save automatic-update policy"
            variant="secondary"
            disabled={blocked || !automaticDirty}
            onPress={() =>
              void write.run(async (current) => {
                await machinesApi.automatic(
                  node,
                  update.automatic,
                  automatic,
                  current,
                );
                if (!current()) return;
                setAutomaticDraft(undefined);
                setNotice(
                  "Automatic-update policy saved. Owner policy is saved separately.",
                );
                await settled();
              })
            }
          />
          {!update.updater_ready && (
            <Notice message="This updater needs manual host steps. Open this machine on the web for verified instructions. Queuing guided update does not perform those steps." />
          )}
          {confirmUpdate ? (
            <>
              <Notice
                message={`Update ${node.name} owned by ${node.owner.display_name} to ${update.target_version}? The machine may restart and interrupt current agent work. A manual step or failure can still be returned.`}
              />
              <ActionButton
                label="Confirm machine update"
                variant="danger"
                disabled={blocked}
                onPress={() =>
                  void write.run(async (current) => {
                    const result = await machinesApi.update(
                      node,
                      update.target_version,
                      current,
                    );
                    if (!current()) return;
                    setConfirmUpdate(false);
                    setNotice(updateResultMessage(result));
                    await settled();
                  })
                }
              />
              <ActionButton
                label="Cancel update"
                variant="secondary"
                disabled={write.busy}
                onPress={() => setConfirmUpdate(false)}
              />
            </>
          ) : (
            <ActionButton
              label="Review machine update"
              variant="secondary"
              disabled={
                blocked ||
                !node.is_connected ||
                !update.update_available ||
                updateInProgress(update)
              }
              onPress={() => setConfirmUpdate(true)}
            />
          )}
        </Card>
      ) : (
        <Notice message="Update status is unavailable. Owner policy can still be reviewed." />
      )}
    </>
  );
}

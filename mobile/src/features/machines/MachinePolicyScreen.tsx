import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type { WorkspaceContentParamList } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { machinesApi } from "../../lib/api/machinesApi";
import {
  confirmDiscard,
  machineError,
  machineKeys,
  useMachineRuntime,
  useMachineWrite,
} from "./MachineUI";
import { machinePageContent, MachineSwitch } from "./MachinePresentation";
import {
  updateInProgress,
  updateResultMessage,
  type MachineNode,
  type MachineUpdate,
} from "./machineContracts";

const CONFIRMATION_OPTIONS = [
  { value: "none",
    get label() {
      return translateApp("No machine confirmation");
    },
  },
  { value: "changes",
    get label() {
      return translateApp("Confirm changes");
    },
  },
  { value: "all",
    get label() {
      return translateApp("Confirm every operation");
    },
  },
] as const satisfies readonly {
  value: MachineNode["machine_confirm"];
  label: string;
}[];

export function MachinePolicyScreen({
  navigation,
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "MachinePolicy">) {
  useAppLanguage();
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
    <WorkspacePage
      title={translateApp("Owner policy and updates")}
      showHeading={false}
      contentStyle={machinePageContent}
    >
      {query.isPending && (
        <SkeletonRows
          rows={3}
          icon={false}
          title
          accessibilityLabel={translateApp("Loading machine authority")}
        />
      )}
      {query.error && (
        <>
          <Notice tone="error" message={machineError(query.error)} />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Refresh Machine Authority")}
              onPress={() => void query.refetch()}
            />
          </SettingsGroup>
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
            if (!result.data) throw localizedError("Machine unavailable");
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
  useAppLanguage();
  const write = useMachineWrite();
  const client = useQueryClient();
  const [baseline, setBaseline] = useState(node);
  const [confirm, setConfirm] = useState(node.machine_confirm);
  const [allow, setAllow] = useState(node.allow_single_user_saved_logins);
  const [ack, setAck] = useState(false);
  const [automaticDraft, setAutomaticDraft] = useState<boolean>();
  const [confirmUpdate, setConfirmUpdate] = useState(false);
  const [notice, setNotice] = useLocalizedState<string>();
  const [refreshError, setRefreshError] = useLocalizedState<string>();
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
            title: translateApp("Discard machine changes?"),
            message: translateApp(
              "Unsaved policy changes and pending update confirmation will be closed.",
            ),
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
      if (write.current()) setRefreshError(() => (machineError(cause)));
    }
  }
  async function settled() {
    await client.invalidateQueries({ queryKey: machineKeys(write.epoch) });
  }
  const savePolicy = () =>
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
        () => (translateApp(
          "Owner policy saved. Automatic-update policy is saved separately.",
        )),
      );
      await settled();
    });
  return (
    <>
      <SettingsGroup
        inset={false}
        footer={translateApp(
          "Owner policy does not change the local capability ceiling or any agent's grant.",
        )}
      >
        <SettingsRow label={translateApp("Machine")} value={node.name} />
        <SettingsRow label={translateApp("Owner")}
          value={node.owner.display_name} />
      </SettingsGroup>
      {write.error && <Notice tone="error" message={write.error} />}
      {refreshError && <Notice tone="error" message={refreshError} />}
      {notice && <Notice message={notice} />}
      <SettingsGroup title={translateApp("Owner policy")} inset={false}>
        <SettingsMenuRow
          label={translateApp("Machine confirmation")}
          value={confirm}
          options={CONFIRMATION_OPTIONS}
          disabled={blocked}
          onChange={setConfirm}
        />
        <MachineSwitch
          label={translateApp("Saved-login typing")}
          detail={translateApp("Allow on a single-user browser")}
          accessibilityLabel={translateApp(
            "Allow saved-login typing on a single-user browser",
          )}
          value={allow}
          disabled={blocked}
          onChange={(value) => {
            setAllow(value);
            setAck(false);
          }}
        />
      </SettingsGroup>
      {needsAck && (
        <>
          <Notice message={translateApp(
              "Commands run as the browser user. A misbehaving or prompt-injected agent could read typed values. Prefer a machine container or a VM with separate browser and agent users.",
            )}
          />
          <SettingsGroup inset={false}>
            <MachineSwitch
              label={translateApp("I acknowledge the risk")}
              detail={translateApp("Typed saved-login values could be read")}
              accessibilityLabel={translateApp(
                "I acknowledge the risk of typed saved-login values being read",
              )}
              value={ack}
              disabled={blocked}
              onChange={setAck}
            />
          </SettingsGroup>
        </>
      )}
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label={write.busy ? translateApp("Saving…")
              : translateApp("Save Owner Policy")
          }
          accessibilityLabel={translateApp("Save owner policy")}
          disabled={blocked || !policyDirty || (needsAck && !ack)}
          onPress={savePolicy}
        />
      </SettingsGroup>
      {update ? (
        <>
          <SettingsGroup
            title={translateApp("Updates")}
            inset={false}
            footer={
              [update.guidance, update.updater_guidance]
                .filter(Boolean)
                .join("\n\n") || undefined
            }
          >
            <SettingsRow
              label={translateApp("Current version")}
              value={update.current_version}
            />
            <SettingsRow label={translateApp("Target version")}
              value={update.target_version} />
            <SettingsRow
              label={translateApp("Phase")}
              value={`${update.phase}${update.code ? ` · ${update.code}` : ""}`}
            />
            <MachineSwitch
              label={translateApp("Automatic machine updates")}
              value={automatic}
              disabled={blocked}
              onChange={setAutomaticDraft}
            />
          </SettingsGroup>
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Save Automatic-Update Policy")}
              accessibilityLabel={translateApp("Save automatic-update policy")}
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
                    () => (translateApp(
                      "Automatic-update policy saved. Owner policy is saved separately.",
                    )),
                  );
                  await settled();
                })
              }
            />
          </SettingsGroup>
          {!update.updater_ready && (
            <Notice message={translateApp(
                "This updater needs manual host steps. Open this machine on the web for verified instructions. Queuing guided update does not perform those steps.",
              )}
            />
          )}
          {confirmUpdate ? (
            <>
              <Notice
                message={translateApp(
                  "Update {{name}} owned by {{organization}} to {{version}}? The machine may restart and interrupt current agent work. A manual step or failure can still be returned.",
                  {
                    name: node.name,
                    organization: node.owner.display_name,
                    version: update.target_version,
                  },
                )}
              />
              <SettingsGroup inset={false}>
                <SettingsRow
                  centered
                  destructive
                  label={translateApp("Confirm Machine Update")}
                  accessibilityLabel={translateApp("Confirm machine update")}
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
                      setNotice(() => (updateResultMessage(result)));
                      await settled();
                    })
                  }
                />
              </SettingsGroup>
              <SettingsGroup inset={false}>
                <SettingsRow
                  centered
                  tone="action"
                  label={translateApp("Cancel Update")}
                  accessibilityLabel={translateApp("Cancel update")}
                  disabled={write.busy}
                  onPress={() => setConfirmUpdate(false)}
                />
              </SettingsGroup>
            </>
          ) : (
            <SettingsGroup inset={false}>
              <SettingsRow
                tone="action"
                label={translateApp("Review Machine Update")}
                accessibilityLabel={translateApp("Review machine update")}
                disabled={
                  blocked ||
                  !node.is_connected ||
                  !update.update_available ||
                  updateInProgress(update)
                }
                onPress={() => setConfirmUpdate(true)}
              />
            </SettingsGroup>
          )}
        </>
      ) : (
        <Notice message={translateApp(
            "Update status is unavailable. Owner policy can still be reviewed.",
          )}
        />
      )}
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label={translateApp("Reload Policy and Update Status")}
          accessibilityLabel={translateApp("Reload policy and update status")}
          disabled={write.busy || !write.active}
          onPress={() => confirmDiscard(dirty, () => void refresh())}
        />
      </SettingsGroup>
    </>
  );
}

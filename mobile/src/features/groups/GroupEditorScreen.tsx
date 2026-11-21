import { discardUploadDraft } from "../assistant/assistantDraftStore";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { AgentAvatar } from "../../components/AgentAvatar";
import { SkeletonRows } from "../../components/Skeleton";
import { useEffect, useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Notice } from "../../components/workspace/WorkspaceUI";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { SettingsSelectionRow } from "../../components/settings/SettingsSelectionRow";
import { groupsApi } from "../../lib/api/groupsApi";
import { agentsApi } from "../../lib/api/agentsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  groupCreateSchema,
  GROUP_MAX_MEMBERS,
  GROUP_MAX_PARTICIPANTS,
  type Group,
  type GroupUpdate,
} from "./groupContracts";
import {
  eligibleGroupAgents,
  groupCanManage,
  groupCanLeave,
} from "./groupState";
import { groupError, groupKeys, useGroupAvailability } from "./useGroupRuntime";
import { CollaborationGroupControls } from "./CollaborationGroupControls";

export type GroupEditorScreenProps = {
  groupId?: string;
  onBack: () => void;
  onSaved: (id: string) => void;
  onRemoved?: () => void;
};

export function GroupEditorScreen(props: GroupEditorScreenProps) {
  useAppLanguage();
  const { epoch } = useGroupAvailability();
  return <GroupEditor key={`${epoch}:${props.groupId ?? "new"}`} {...props} />;
}

function GroupEditor({
  groupId,
  onBack,
  onSaved,
  onRemoved,
}: GroupEditorScreenProps) {
  useAppLanguage();
  const live = useGroupAvailability();
  const client = useQueryClient();
  const scope = useRef({ mounted: true, epoch: live.epoch });
  const lock = useRef(false);
  const exitConsumed = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useLocalizedState<string>();
  const [baseline, setBaseline] = useState<Group>();
  const [initialized, setInitialized] = useState(false);
  const [name, setName] = useState("");
  const [org, setOrg] = useState<string>();
  const [members, setMembers] = useState<string[]>([]);
  const [people, setPeople] = useState<string[]>([]);
  const [lead, setLead] = useState<string>();
  const [memberTouched, setMemberTouched] = useState(false);
  const [peopleTouched, setPeopleTouched] = useState(false);
  const [exitTo, setExitTo] = useState<
    { kind: "back" | "removed" } | { kind: "saved"; id: string }
  >();
  const agents = useQuery({
    queryKey: ["workspace", "agents", "group-picker", live.epoch],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: live.active,
    retry: false,
  });
  const detail = useQuery({
    queryKey: [...groupKeys(live.epoch), "detail", groupId],
    queryFn: ({ signal }) => groupsApi.detail(groupId!, signal),
    enabled: live.active && !!groupId,
    retry: false,
  });
  const orgs = useQuery({
    queryKey: [...groupKeys(live.epoch), "organizations"],
    queryFn: ({ signal }) => groupsApi.orgs(signal),
    enabled: live.active && !groupId,
    retry: false,
  });
  const features = useQuery({
    queryKey: [...groupKeys(live.epoch), "features"],
    queryFn: ({ signal }) => groupsApi.features(signal),
    enabled: live.active && !groupId,
    retry: false,
  });
  const identity = useQuery({
    queryKey: [...groupKeys(live.epoch), "identity"],
    queryFn: ({ signal }) => groupsApi.identity(signal),
    enabled: live.active && !groupId,
    retry: false,
  });
  const participants = useQuery({
    queryKey: [...groupKeys(live.epoch), "participants", org],
    queryFn: ({ signal }) => groupsApi.orgMembers(org!, signal),
    enabled: live.active && !!org,
    retry: false,
  });
  const allAgents = agents.data?.agents ?? [];
  const eligible = eligibleGroupAgents(allAgents, org);
  const currentGroup = detail.isError ? undefined : (detail.data ?? baseline);
  const canManage =
    !groupId || (!!currentGroup && groupCanManage(currentGroup));
  const enabledOrgs = (orgs.data ?? []).filter((row) =>
    ["admin", "member"].includes(row.your_role),
  );
  const orgEnabled = features.data?.includes("assistant:org-agents") === true;
  const availablePeople = (participants.data ?? []).filter(
    (row) => !row.revoked_at && ["admin", "member"].includes(row.role),
  );
  const aliveIds = new Set(eligible.map((agent) => agent.id));
  const gone = !!baseline?.members.some(
    (member) => member.destroyed || !aliveIds.has(member.id),
  );
  const dirty = !groupId
    ? !!name || memberTouched || !!org
    : !!baseline &&
      (name !== baseline.name ||
        memberTouched ||
        peopleTouched ||
        lead !== (org ? baseline.lead_agent_id : undefined) ||
        gone);
  const selectionDisabled =
    busy || !live.active || agents.isError || detail.isError;
  const disabled = selectionDisabled || !canManage;
  useEffect(() => {
    scope.current.mounted = true;
    return () => {
      scope.current.mounted = false;
    };
  }, []);
  useEffect(() => {
    if (initialized || !agents.data || (groupId && !detail.data)) return;
    const group = detail.data;
    if (group) {
      setBaseline(group);
      setName(group.name);
      setOrg(group.owner?.type === "org" ? group.owner.id : undefined);
      const known = new Set(
        eligibleGroupAgents(
          allAgents,
          group.owner?.type === "org" ? group.owner.id : undefined,
        ).map((agent) => agent.id),
      );
      setMembers(
        group.members
          .filter((member) => !member.destroyed && known.has(member.id))
          .map((member) => member.id),
      );
      setPeople(group.participants?.map((person) => person.id) ?? []);
      setLead(group.owner?.type === "org" ? group.lead_agent_id : undefined);
    } else
      setMembers(
        allAgents
          .filter(
            (agent) =>
              agent.kind === "nyxbot" &&
              agent.owner_kind !== "org" &&
              agent.status !== "destroyed",
          )
          .slice(0, 1)
          .map((agent) => agent.id),
      );
    setInitialized(true);
  }, [initialized, agents.data, detail.data, groupId, allAgents]);

  function back() {
    if (busy) return;
    if (!dirty) setExitTo({ kind: "back" });
    else
      Alert.alert(
        translateApp("Discard crew changes?"),
        translateApp("Your changes have not been saved."),
        [
          { text: translateApp("Keep editing"), style: "cancel" },
          {
            text: translateApp("Discard"),
            style: "destructive",
            onPress: () => setExitTo({ kind: "back" }),
          },
        ],
      );
  }
  useWorkspaceBack({
    active: !exitTo && (dirty || busy),
    defaultRole: "back",
    disabled: !exitTo && busy,
    label: groupId ? translateApp("Back to crew")
      : translateApp("Back to crews"),
    onBack: back,
    discardOnClose:
      !exitTo && dirty
        ? {
            title: translateApp("Discard crew changes?"),
            message: translateApp("Your changes have not been saved."),
          }
        : undefined,
  });
  useEffect(() => {
    if (!exitTo || busy || !live.active || exitConsumed.current) return;
    exitConsumed.current = true;
    if (exitTo.kind === "saved") onSaved(exitTo.id);
    else if (exitTo.kind === "removed") (onRemoved ?? onBack)();
    else onBack();
  }, [exitTo, busy, live.active, onSaved, onRemoved, onBack]);
  async function perform(action: () => Promise<void>) {
    if (lock.current || exitConsumed.current || exitTo || !live.available()) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (cause) {
      if (scope.current.mounted && live.available())
        setError(() => (groupError(cause)));
    } finally {
      lock.current = false;
      if (scope.current.mounted) setBusy(false);
    }
  }
  async function save() {
    if (!canManage || agents.isError || detail.isError || !initialized) return;
    await perform(async () => {
      const selectedMembers = members.filter((id) => aliveIds.has(id));
      const valid = groupCreateSchema.safeParse({
        name,
        member_agent_ids: selectedMembers,
        ...(org ? { org, participant_user_ids: people } : {}),
      });
      if (!valid.success)
        throw new Error(
          valid.error.issues[0]?.message ??
            translateApp("Review the crew details."),
        );
      let saved: Group;
      if (!groupId) {
        if (org && (!orgEnabled || !enabledOrgs.some((row) => row.id === org)))
          throw localizedError("You cannot create a crew for this organization.");
        saved = await groupsApi.create(valid.data);
      } else {
        const patch: GroupUpdate = {};
        if (name.trim() !== baseline!.name) patch.name = name.trim();
        if (memberTouched || gone) patch.member_agent_ids = selectedMembers;
        if (org && peopleTouched) patch.participant_user_ids = people;
        if (org && (!lead || !selectedMembers.includes(lead)))
          throw localizedError("Choose a lead from the selected agents.");
        if (org && lead !== baseline!.lead_agent_id) {
          if (!lead || !selectedMembers.includes(lead))
            throw localizedError("Choose a lead from the selected agents.");
          patch.lead_agent_id = lead;
        }
        const latest = await groupsApi.detail(groupId);
        if (!groupCanManage(latest))
          throw localizedError("You can no longer manage this crew. Reopen it to review access.");
        const ids = (values: readonly string[]) =>
          [...values].sort().join("\n");
        if (
          (patch.name !== undefined && latest.name !== baseline!.name) ||
          (patch.member_agent_ids &&
            ids(latest.members.map((row) => row.id)) !==
              ids(baseline!.members.map((row) => row.id))) ||
          (patch.participant_user_ids &&
            ids(latest.participants?.map((row) => row.id) ?? []) !==
              ids(baseline!.participants?.map((row) => row.id) ?? [])) ||
          (patch.lead_agent_id &&
            latest.lead_agent_id !== baseline!.lead_agent_id)
        ) {
          throw localizedError("This crew changed elsewhere. Your draft is kept. Go back and reopen settings to review the latest changes.");
        }
        if (!live.available())
          throw localizedError("Reconnect and keep crew settings open to save.");
        saved = await groupsApi.update(groupId, patch);
      }
      if (scope.current.mounted && scope.current.epoch === getAuthEpoch()) {
        await client.invalidateQueries({ queryKey: groupKeys(live.epoch) });
        if (scope.current.mounted && scope.current.epoch === getAuthEpoch())
          setExitTo({ kind: "saved", id: saved.id });
      }
    });
  }
  function confirmDestructive(kind: "leave" | "delete") {
    if (
      !baseline ||
      !currentGroup ||
      detail.isError ||
      busy ||
      !live.available() ||
      (kind === "delete" ? !canManage : !groupCanLeave(currentGroup))
    )
      return;
    Alert.alert(
      kind === "delete" ? translateApp("Delete {{name}}?", { name: baseline.name })
        : translateApp("Leave crew?"),
      kind === "delete"
        ? translateApp(
            "The crew and its messages will be deleted. Your agents and their own threads stay. This cannot be undone.",
          )
        : translateApp(
            "The creator or a participating Admin must remain. Leaving as the last participant deletes the crew. Wait for your running turns to finish.",
          ),
      [
        { text: translateApp("Cancel"), style: "cancel" },
        {
          text: kind === "delete" ? translateApp("Delete crew")
              : translateApp("Leave crew"),
          style: "destructive",
          onPress: () =>
            void perform(async () => {
              const owner = getAuthAccountId();
              if (kind === "delete") await groupsApi.remove(baseline.id);
              else await groupsApi.leave(baseline.id);
              void discardUploadDraft(owner, `crew:${baseline.id}`).catch(() => {});
              if (
                scope.current.mounted &&
                scope.current.epoch === getAuthEpoch()
              ) {
                client.removeQueries({
                  queryKey: [...groupKeys(live.epoch), "messages", baseline.id],
                  exact: true,
                });
                await client.invalidateQueries({
                  queryKey: groupKeys(live.epoch),
                });
                if (
                  scope.current.mounted &&
                  scope.current.epoch === getAuthEpoch()
                )
                  setExitTo({ kind: "removed" });
              }
            }),
        },
      ],
    );
  }
  if (detail.data?.collaboration)
    return (
      <CollaborationGroupControls
        group={detail.data}
        onSaved={onSaved}
        onRemoved={onRemoved ?? onBack}
      />
    );
  const removedMembers =
    baseline?.members.filter(
      (member) => member.destroyed || !aliveIds.has(member.id),
    ) ?? [];
  return (
    <SettingsPage>
      {error && <Notice message={error} tone="error" />}
      {(agents.isPending || (!!groupId && detail.isPending)) && (
        <SkeletonRows rows={3} title accessibilityLabel={translateApp("Loading crew editor")}
        />
      )}
      {(agents.isError || detail.isError) && (
        <>
          <Notice
            message={translateApp(
              "Could not load the crew's agents and settings.",
            )}
            tone="error"
          />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Retry")}
              accessibilityLabel={translateApp("Retry crew editor")}
              onPress={() => {
                void agents.refetch();
                if (groupId) void detail.refetch();
              }}
            />
          </SettingsGroup>
        </>
      )}
      {initialized && (
        <>
          {baseline?.owner?.type === "org" && (
            <Notice
              message={(canManage ? translateApp("{{name}} · You can manage this crew.", { name: baseline.owner.name }) : translateApp("{{name}} · Only the creator and participating Admins manage this crew.", { name: baseline.owner.name }))}
            />
          )}
          <SettingsGroup
            inset={false}
            footer={
              !groupId && (orgs.isError || features.isError)
                ? translateApp(
                    "Organization choices could not be loaded. Personal crews remain available.",
                  )
                : !groupId && !!enabledOrgs.length && !orgEnabled
                  ? translateApp("Organization crews are not enabled.")
                  : !groupId
                    ? translateApp(
                        "Changing ownership resets the selected agents and participants.",
                      )
                    : undefined
            }
          >
            <SettingsTextField
              accessibilityLabel={translateApp("Name")}
              value={name}
              onChangeText={setName}
              placeholder={translateApp("Crew name, e.g. Launch crew")}
              autoCapitalize="sentences"
              editable={!disabled}
            />
            {!groupId && (
              <SettingsSelectionRow
                label={translateApp("Ownership")}
                title={translateApp("Choose crew owner")}
                description={translateApp(
                  "Choose where this crew belongs. Changing ownership resets its selected agents and participants.",
                )}
                options={[
                  { value: "personal", label: translateApp("Personal") },
                  ...enabledOrgs.map((row) => ({
                    value: row.id,
                    label: row.display_name ?? row.slug,
                    description: row.slug,
                    disabled: row.id !== org && (!orgEnabled || !identity.data),
                  })),
                ]}
                value={[org ?? "personal"]}
                disabled={busy || !live.active}
                scopeKey={`${live.epoch}:new-group-owner`}
                confirmLabel={translateApp("Use this owner")}
                onChange={([owner]) => {
                  const nextOrg = owner === "personal" ? undefined : owner;
                  if (nextOrg === org) return;
                  setOrg(nextOrg);
                  setMembers(
                    nextOrg
                      ? []
                      : allAgents
                          .filter(
                            (agent) =>
                              agent.kind === "nyxbot" &&
                              agent.owner_kind !== "org" &&
                              agent.status !== "destroyed",
                          )
                          .slice(0, 1)
                          .map((agent) => agent.id),
                  );
                  setPeople(nextOrg && identity.data ? [identity.data.id] : []);
                  setLead(undefined);
                  setMemberTouched(false);
                  setPeopleTouched(false);
                }}
              />
            )}
          </SettingsGroup>
          <SettingsGroup
            inset={false}
            title={translateApp("Members")}
            footer={
              !eligible.length
                ? translateApp("No eligible agents available.")
                : translateApp(
                    "@mention agents or choose recipients in the chat. Otherwise, messages go to whoever you last addressed, then the lead agent.",
                  )
            }
          >
            <SettingsSelectionRow
              label={translateApp("Members")}
              title={translateApp("Choose crew agents")}
              description={translateApp(
                "Choose up to {{count}} agents. Apply updates this draft; save the crew to keep your changes.",
                { count: GROUP_MAX_MEMBERS },
              )}
              options={eligible.map((agent) => ({
                value: agent.id,
                label: agent.display_name ?? agent.name,
                leading: <AgentAvatar agent={agent} size="lg" />,
                description:
                  agent.kind === "nyxbot"
                    ? translateApp("Your personal agent")
                    : agent.description || translateApp("Specialist"),
                searchText: agent.name,
              }))}
              value={members}
              multiple
              maxSelected={GROUP_MAX_MEMBERS}
              disabled={selectionDisabled}
              readOnly={!canManage}
              scopeKey={`${live.epoch}:${groupId ?? "new"}:${org ?? "personal"}:agents`}
              placeholder={translateApp("Choose agents")}
              confirmLabel={translateApp("Use these agents")}
              onChange={(ids) => {
                setMemberTouched(true);
                setMembers(ids);
              }}
            />
            {removedMembers.map((member) => (
              <SettingsRow
                key={member.id}
                label={member.name}
                detail={translateApp(
                  "No longer available; removed when you save.",
                )}
                disabled
              />
            ))}
          </SettingsGroup>
          {!!org && (
            <SettingsGroup
              inset={false}
              title={translateApp("Organization")}
              footer={
                participants.isPending
                  ? translateApp("Loading organization members…")
                  : undefined
              }
            >
              <SettingsSelectionRow
                label={translateApp("Participants")}
                title={translateApp("Choose crew participants")}
                description={translateApp(
                  "Choose up to {{count}} organization Admins and Members. The creator is included when creating a crew.",
                  { count: GROUP_MAX_PARTICIPANTS },
                )}
                options={[
                  ...availablePeople.map((person) => ({
                    value: person.user_id,
                    label: person.display_name ?? person.email ??
                      translateApp("Member"),
                    description: person.email
                      ? `${person.email} · ${person.role}`
                      : person.role,
                    disabled: !groupId && person.user_id === identity.data?.id,
                  })),
                  ...people
                    .filter(
                      (id) =>
                        !availablePeople.some(
                          (person) => person.user_id === id,
                        ),
                    )
                    .map((id) => ({
                      value: id,
                      label:
                        baseline?.participants?.find(
                          (person) => person.id === id,
                        )?.display_name ?? translateApp("Selected participant"),
                      description:
                        !groupId && id === identity.data?.id
                          ? translateApp(
                              "Creator · included when creating a crew",
                            )
                          : translateApp(
                              "No longer listed as an eligible organization member. Remove this participant before saving.",
                            ),
                      disabled: !groupId && id === identity.data?.id,
                    })),
                ]}
                value={people}
                multiple
                maxSelected={GROUP_MAX_PARTICIPANTS}
                disabled={
                  selectionDisabled ||
                  participants.isPending ||
                  participants.isError
                }
                readOnly={!canManage}
                scopeKey={`${live.epoch}:${groupId ?? "new"}:${org}:participants`}
                placeholder={translateApp("Choose participants")}
                confirmLabel={translateApp("Use these participants")}
                onChange={(ids) => {
                  setPeopleTouched(true);
                  setPeople(ids);
                }}
              />
              {!!groupId && (
                <SettingsSelectionRow
                  label={translateApp("Lead agent")}
                  title={translateApp("Choose the lead agent")}
                  description={translateApp(
                    "Messages use the lead agent when no recipient or previous recipient is selected. Choose from this crew's selected agents.",
                  )}
                  options={eligible
                    .filter((agent) => members.includes(agent.id))
                    .map((agent) => ({
                      value: agent.id,
                      label: agent.display_name ?? agent.name,
                      leading: <AgentAvatar agent={agent} size="lg" />,
                      description: agent.description || undefined,
                      searchText: agent.name,
                    }))}
                  value={lead && members.includes(lead) ? [lead] : []}
                  disabled={selectionDisabled || !members.length}
                  readOnly={!canManage}
                  scopeKey={`${live.epoch}:${groupId}:${org}:lead:${members.join(",")}`}
                  placeholder={translateApp("Choose")}
                  confirmLabel={translateApp("Use this lead agent")}
                  onChange={([id]) => setLead(id)}
                />
              )}
            </SettingsGroup>
          )}
          {!!org && participants.isError && (
            <Notice
              message={translateApp("Could not load organization members.")}
              tone="error"
            />
          )}
          {canManage && (
            <SettingsGroup inset={false}>
              <SettingsRow
                tone="action"
                centered
                label={
                  busy
                    ? groupId
                      ? translateApp("Saving…")
                      : translateApp("Creating…")
                    : groupId
                      ? translateApp("Save changes")
                      : translateApp("Create crew")
                }
                accessibilityLabel={groupId ? translateApp("Save changes")
                    : translateApp("Create crew")
                }
                onPress={() => void save()}
                disabled={
                  busy ||
                  !live.active ||
                  !name.trim() ||
                  !members.length ||
                  agents.isError ||
                  detail.isError ||
                  (!!groupId && !dirty) ||
                  (!!org && participants.isError)
                }
              />
            </SettingsGroup>
          )}
          {baseline && currentGroup && groupCanLeave(currentGroup) && (
            <SettingsGroup inset={false}>
              <SettingsRow
                tone="danger"
                label={translateApp("Leave crew")}
                disabled={busy || !live.active}
                onPress={() => confirmDestructive("leave")}
              />
            </SettingsGroup>
          )}
          {baseline && canManage && (
            <SettingsGroup
              inset={false}
              footer={translateApp(
                "Deletes the crew and its messages. Your agents and their own threads stay.",
              )}
            >
              <SettingsRow
                centered
                destructive
                label={translateApp("Delete Crew")}
                accessibilityLabel={translateApp("Delete crew")}
                disabled={busy || !live.active}
                onPress={() => confirmDestructive("delete")}
              />
            </SettingsGroup>
          )}
        </>
      )}
    </SettingsPage>
  );
}

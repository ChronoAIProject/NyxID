import { AgentAvatar } from "../../components/AgentAvatar";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { groupsApi } from "../../lib/api/groupsApi";
import { agentsApi } from "../../lib/api/agentsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
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

export type GroupEditorScreenProps = {
  groupId?: string;
  onBack: () => void;
  onSaved: (id: string) => void;
  onRemoved?: () => void;
};

export function GroupEditorScreen(props: GroupEditorScreenProps) {
  const { epoch } = useGroupAvailability();
  return <GroupEditor key={`${epoch}:${props.groupId ?? "new"}`} {...props} />;
}

function GroupEditor({
  groupId,
  onBack,
  onSaved,
  onRemoved,
}: GroupEditorScreenProps) {
  const { colors } = useTheme();
  const live = useGroupAvailability();
  const client = useQueryClient();
  const scope = useRef({ mounted: true, epoch: live.epoch });
  const lock = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
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
        "Discard crew changes?",
        "Your changes have not been saved.",
        [
          { text: "Keep editing", style: "cancel" },
          {
            text: "Discard",
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
    label: groupId ? "Back to crew" : "Back to crews",
    onBack: back,
    discardOnClose:
      !exitTo && dirty
        ? {
            title: "Discard crew changes?",
            message: "Your changes have not been saved.",
          }
        : undefined,
  });
  useEffect(() => {
    if (!exitTo || busy || !live.active) return;
    if (exitTo.kind === "saved") onSaved(exitTo.id);
    else if (exitTo.kind === "removed") (onRemoved ?? onBack)();
    else onBack();
  }, [exitTo, busy, live.active, onSaved, onRemoved, onBack]);
  async function perform(action: () => Promise<void>) {
    if (lock.current || !live.available()) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (cause) {
      if (scope.current.mounted && live.available())
        setError(groupError(cause));
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
          valid.error.issues[0]?.message ?? "Review the crew details.",
        );
      let saved: Group;
      if (!groupId) {
        if (org && (!orgEnabled || !enabledOrgs.some((row) => row.id === org)))
          throw new Error("You cannot create a crew for this organization.");
        saved = await groupsApi.create(valid.data);
      } else {
        const patch: GroupUpdate = {};
        if (name.trim() !== baseline!.name) patch.name = name.trim();
        if (memberTouched || gone) patch.member_agent_ids = selectedMembers;
        if (org && peopleTouched) patch.participant_user_ids = people;
        if (org && (!lead || !selectedMembers.includes(lead)))
          throw new Error("Choose a lead from the selected agents.");
        if (org && lead !== baseline!.lead_agent_id) {
          if (!lead || !selectedMembers.includes(lead))
            throw new Error("Choose a lead from the selected agents.");
          patch.lead_agent_id = lead;
        }
        const latest = await groupsApi.detail(groupId);
        if (!groupCanManage(latest))
          throw new Error(
            "You can no longer manage this crew. Reopen it to review access.",
          );
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
          throw new Error(
            "This crew changed elsewhere. Your draft is kept. Go back and reopen settings to review the latest changes.",
          );
        }
        if (!live.available())
          throw new Error("Reconnect and keep crew settings open to save.");
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
      kind === "delete" ? `Delete ${baseline.name}?` : "Leave crew?",
      kind === "delete"
        ? "The crew and its messages will be deleted. Your agents and their own threads stay. This cannot be undone."
        : "The creator or a participating Admin must remain. Leaving as the last participant deletes the crew. Wait for your running turns to finish.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: kind === "delete" ? "Delete crew" : "Leave crew",
          style: "destructive",
          onPress: () =>
            void perform(async () => {
              if (kind === "delete") await groupsApi.remove(baseline.id);
              else await groupsApi.leave(baseline.id);
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
  return (
    <WorkspacePage
      showHeading={false}
      title={groupId ? "Crew settings" : "New crew"}
    >
      {error && <Notice message={error} tone="error" />}
      {(agents.isPending || (!!groupId && detail.isPending)) && (
        <ActivityIndicator
          accessibilityLabel="Loading crew editor"
          color={colors.primary}
        />
      )}
      {(agents.isError || detail.isError) && (
        <>
          <Notice
            message="Could not load the crew's agents and settings."
            tone="error"
          />
          <ActionButton
            label="Retry crew editor"
            variant="secondary"
            onPress={() => {
              void agents.refetch();
              if (groupId) void detail.refetch();
            }}
          />
        </>
      )}
      {initialized && (
        <>
          {!groupId && (
            <Card>
              <WorkspaceSelectionField
                label="Ownership"
                title="Choose crew owner"
                description="Choose where this crew belongs. Changing ownership resets its selected agents and participants."
                options={[
                  { value: "personal", label: "Personal" },
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
                confirmLabel="Use this owner"
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
              {(orgs.isError || features.isError) && (
                <Label muted>
                  Organization choices could not be loaded. Personal crews
                  remain available.
                </Label>
              )}
              {!!enabledOrgs.length && !orgEnabled && (
                <Label muted>Organization crews are not enabled.</Label>
              )}
            </Card>
          )}
          {baseline?.owner?.type === "org" && (
            <Notice
              message={`${baseline.owner.name} · ${canManage ? "You can manage this crew." : "Only the creator and participating Admins manage this crew."}`}
            />
          )}
          <Field
            label="Name"
            value={name}
            onChangeText={setName}
            placeholder="Launch crew"
            editable={!disabled}
          />
          <Card>
            <WorkspaceSelectionField
              label="Members"
              title="Choose crew agents"
              description={`Choose up to ${GROUP_MAX_MEMBERS} agents. Apply updates this draft; save the crew to keep your changes.`}
              options={eligible.map((agent) => ({
                value: agent.id,
                label: agent.display_name ?? agent.name,
                leading: <AgentAvatar agent={agent} size="lg" />,
                description:
                  agent.kind === "nyxbot"
                    ? "Your personal agent"
                    : agent.description || "Specialist",
                searchText: agent.name,
              }))}
              value={members}
              multiple
              maxSelected={GROUP_MAX_MEMBERS}
              disabled={selectionDisabled}
              readOnly={!canManage}
              scopeKey={`${live.epoch}:${groupId ?? "new"}:${org ?? "personal"}:agents`}
              placeholder="Choose agents"
              confirmLabel="Use these agents"
              onChange={(ids) => {
                setMemberTouched(true);
                setMembers(ids);
              }}
            />
            {baseline?.members
              .filter((member) => member.destroyed || !aliveIds.has(member.id))
              .map((member) => (
                <Label key={member.id} muted>
                  {member.name} · No longer available; removed when you save.
                </Label>
              ))}
            {!eligible.length && (
              <Label muted>No eligible agents available.</Label>
            )}
          </Card>
          {!!org && (
            <Card>
              <WorkspaceSelectionField
                label="Participants"
                title="Choose crew participants"
                description={`Choose up to ${GROUP_MAX_PARTICIPANTS} organization Admins and Members. The creator is included when creating a crew.`}
                options={[
                  ...availablePeople.map((person) => ({
                    value: person.user_id,
                    label: person.display_name ?? person.email ?? "Member",
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
                        )?.display_name ?? "Selected participant",
                      description:
                        !groupId && id === identity.data?.id
                          ? "Creator · included when creating a crew"
                          : "No longer listed as an eligible organization member. Remove this participant before saving.",
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
                placeholder="Choose participants"
                confirmLabel="Use these participants"
                onChange={(ids) => {
                  setPeopleTouched(true);
                  setPeople(ids);
                }}
              />
              {participants.isPending && (
                <ActivityIndicator color={colors.primary} />
              )}
              {participants.isError && (
                <Notice
                  message="Could not load organization members."
                  tone="error"
                />
              )}
              {!!groupId && (
                <WorkspaceSelectionField
                  label="Lead agent"
                  title="Choose the lead agent"
                  description="Messages use the lead agent when no recipient or previous recipient is selected. Choose from this crew's selected agents."
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
                  placeholder="Choose a lead agent"
                  confirmLabel="Use this lead agent"
                  onChange={([id]) => setLead(id)}
                />
              )}
            </Card>
          )}
          <Label muted>
            @mention agents or choose recipients in the chat. Otherwise,
            messages go to whoever you last addressed, then the lead agent.
          </Label>
          {canManage && (
            <ActionButton
              label={groupId ? "Save changes" : "Create crew"}
              onPress={() => void save()}
              loading={busy}
              disabled={
                !live.active ||
                !name.trim() ||
                !members.length ||
                agents.isError ||
                detail.isError ||
                (!!groupId && !dirty) ||
                (!!org && participants.isError)
              }
            />
          )}
          {baseline && currentGroup && groupCanLeave(currentGroup) && (
            <ActionButton
              label="Leave crew"
              variant="secondary"
              disabled={busy || !live.active}
              onPress={() => confirmDestructive("leave")}
            />
          )}
          {baseline && canManage && (
            <ActionButton
              label="Delete crew"
              variant="danger"
              disabled={busy || !live.active}
              onPress={() => confirmDestructive("delete")}
            />
          )}
        </>
      )}
    </WorkspacePage>
  );
}

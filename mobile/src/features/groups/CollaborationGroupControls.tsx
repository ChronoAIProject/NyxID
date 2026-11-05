import { useEffect, useState } from "react";
import { Alert, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { useQuery } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { SkeletonRows } from "../../components/Skeleton";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { groupsApi } from "../../lib/api/groupsApi";
import { agentsApi } from "../../lib/api/agentsApi";
import { collaborationApi } from "../../lib/api/collaborationApi";
import {
  inviteToken,
  invitationLink,
} from "../../lib/api/collaborationTransport";
import { getFrontendUrl } from "../../lib/env";
import { useCollaborationSession } from "./useCollaborationSession";
import type { Group, GroupMember } from "./groupContracts";
import type {
  CollaborationChange,
  CollaborationDecision,
} from "./collaborationContracts";

export function CollaborationGroupControls({
  group,
  onSaved,
  onRemoved,
}: {
  group: Group;
  onSaved: (id: string) => void;
  onRemoved: () => void;
}) {
  const session = useCollaborationSession();
  const detail = useQuery({
    queryKey: [...session.key, "group", group.id],
    queryFn: ({ signal }) => groupsApi.detail(group.id, signal),
    enabled: session.active,
    retry: false,
    refetchInterval: session.active ? 30_000 : false,
  });
  const agents = useQuery({
    queryKey: [...session.key, "agents"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: session.active,
    retry: false,
  });
  const orgs = useQuery({
    queryKey: [...session.key, "orgs"],
    queryFn: ({ signal }) => groupsApi.orgs(signal),
    enabled: session.active,
    retry: false,
  });
  // The 30s poll refreshes silently; only the person's pull shows the spinner.
  const pull = usePullRefresh(() =>
    Promise.all([detail.refetch(), session.identity.refetch()]),
  );
  const [org, setOrg] = useState("");
  const people = useQuery({
    queryKey: [...session.key, "org-people", org],
    queryFn: ({ signal }) => groupsApi.orgMembers(org, signal),
    enabled: session.active && !!org,
    retry: false,
  });
  const [name, setName] = useState(group.name);
  const [invite, setInvite] = useState<{ token: string; expires_at: string }>();
  useEffect(() => {
    if (!session.active) setInvite(undefined);
  }, [session.active, session.epoch]);
  useEffect(() => {
    setInvite(undefined);
  }, [session.epoch]);
  const current =
    detail.isError ||
    !session.readable ||
    detail.dataUpdatedAt < session.activationAt
      ? undefined
      : detail.data;
  const policy = current?.collaboration;
  const disabled = session.busy || !session.active || !policy;
  const apply = (change: CollaborationChange) =>
    void session.run(
      async () => {
        if (!policy) throw new Error("Reload group settings first.");
        const fresh = await groupsApi.detail(group.id);
        if (!session.valid())
          throw new Error("Keep group settings open to save.");
        if (
          fresh.collaboration?.membership_revision !==
          policy.membership_revision
        )
          throw new Error(
            "This group changed elsewhere. Refresh and review its current participants before saving.",
          );
        return collaborationApi.change(
          group.id,
          policy.membership_revision,
          change,
          { authorityGuard: session.guard },
        );
      },
      (result) => {
        if (result.invite) setInvite(result.invite);
        if (
          !result.group &&
          (change.action === "leave" || change.action === "delete")
        )
          onRemoved();
      },
      {
        feature:
          [
            "leave",
            "delete",
            "remove",
            "remove_agent",
            "cancel_invite",
          ].includes(change.action) ||
          (change.action === "guest_access" && !change.enabled)
            ? undefined
            : "assistant:shared-agents",
        capability:
          ![
            "leave",
            "delete",
            "remove",
            "remove_agent",
            "cancel_invite",
          ].includes(change.action) &&
          !(change.action === "guest_access" && !change.enabled),
      },
    );
  const confirm = (message: string, change: CollaborationChange) =>
    Alert.alert("Confirm group change", message, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Confirm",
        style:
          change.action === "delete" ||
          change.action === "remove" ||
          change.action === "remove_agent"
            ? "destructive"
            : "default",
        onPress: () => apply(change),
      },
    ]);
  if (!session.active)
    return (
      <WorkspacePage title="Group settings">
        <Label muted>Return to this view to reload current group rights.</Label>
      </WorkspacePage>
    );
  return (
    <WorkspacePage
      title="Group participants and agents"
      onRefresh={pull.onRefresh}
      refreshing={pull.refreshing}
    >
      {session.error && <Notice tone="error" message={session.error} />}
      {detail.isError && (
        <Notice
          tone="error"
          message="Group access changed or settings could not load. Refresh to review. Actions are disabled."
        />
      )}
      {policy && current ? (
        <>
          <Notice message={policy.history_disclosure} />
          {policy.can_manage && (
            <Card>
              <Field
                label="Group name"
                value={name}
                onChangeText={setName}
                editable={!disabled}
                maxLength={60}
              />
              <ActionButton
                label="Save group name"
                disabled={disabled || !name.trim() || name === current.name}
                onPress={() => apply({ action: "rename", name: name.trim() })}
              />
            </Card>
          )}
          <Card>
            <Label>
              Participants ({current.participants?.length ?? 0} of 16)
            </Label>
            {current.participants?.map((person) => (
              <View key={person.id} style={{ gap: 8 }}>
                <Label>
                  {person.display_name}
                  {person.id === policy.creator_user_id ? " · Creator" : ""}
                </Label>
                {policy.can_manage && person.id !== policy.creator_user_id && (
                  <>
                    <ActionButton
                      compact
                      label={`Transfer management to ${person.display_name}`}
                      disabled={disabled}
                      onPress={() =>
                        confirm(
                          "They must accept management before you can leave. This does not transfer any agent or credentials.",
                          { action: "propose_transfer", person_id: person.id },
                        )
                      }
                    />
                    <ActionButton
                      compact
                      variant="danger"
                      label={`Remove ${person.display_name}`}
                      disabled={disabled}
                      onPress={() =>
                        confirm(
                          "Their group access ends immediately and queued work is cancelled.",
                          { action: "remove", person_id: person.id },
                        )
                      }
                    />
                  </>
                )}
              </View>
            ))}
            {policy.transfer_to_user_id === session.identity.data?.id ? (
              <ActionButton
                label="Accept group management"
                disabled={disabled}
                onPress={() =>
                  confirm(
                    "You will manage group participants and invitations. Agent ownership stays unchanged.",
                    { action: "accept_transfer" },
                  )
                }
              />
            ) : policy.transfer_to_user_id ? (
              <Label muted>
                Waiting for the proposed creator to accept management.
              </Label>
            ) : null}
          </Card>
          {policy.can_manage && (
            <Card>
              <ActionButton
                label="Create invitation link"
                disabled={disabled || (current.participants?.length ?? 0) >= 16}
                onPress={() => apply({ action: "invite" })}
              />
              <Label muted>
                Single use, expires in seven days. Share it yourself. Joining
                exposes the full retained group history.
              </Label>
              {invite && (
                <>
                  <Label selectable>
                    {invitationLink(getFrontendUrl(), invite.token) ??
                      invite.token}
                  </Label>
                  <Label muted>
                    Expires {new Date(invite.expires_at).toLocaleString()}
                  </Label>
                  <ActionButton
                    label={
                      invitationLink(getFrontendUrl(), invite.token)
                        ? "Copy invitation link"
                        : "Copy invitation token"
                    }
                    disabled={disabled}
                    onPress={() =>
                      void Clipboard.setStringAsync(
                        invitationLink(getFrontendUrl(), invite.token) ??
                          invite.token,
                      )
                    }
                  />
                </>
              )}
              {policy.invites.map((row) => (
                <ActionButton
                  key={row.id}
                  variant="secondary"
                  label={`Revoke invitation · ${new Date(row.expires_at).toLocaleDateString()}`}
                  disabled={disabled}
                  onPress={() =>
                    apply({ action: "cancel_invite", invite_id: row.id })
                  }
                />
              ))}
              <WorkspaceSelectionField
                label="Quick-add from organization"
                title="Choose a shared organization"
                options={(orgs.data ?? []).map((row) => ({
                  value: row.id,
                  label: row.display_name ?? row.slug,
                }))}
                value={org ? [org] : []}
                onChange={([id]) => setOrg(id ?? "")}
                disabled={disabled}
                scopeKey={`${session.epoch}:quick-add-org`}
              />
              {people.isError && (
                <Notice
                  tone="error"
                  message="Organization members could not load."
                />
              )}
              {(people.data ?? [])
                .filter(
                  (p) =>
                    !p.revoked_at &&
                    !current.participants?.some((row) => row.id === p.user_id),
                )
                .map((person) => (
                  <ActionButton
                    key={person.user_id}
                    compact
                    label={`Add ${person.display_name ?? person.email ?? "member"}`}
                    disabled={
                      disabled || (current.participants?.length ?? 0) >= 16
                    }
                    onPress={() =>
                      confirm(
                        "This person will immediately see the full retained group history.",
                        {
                          action: "quick_add",
                          person_id: person.user_id,
                          history_disclosed: true,
                        },
                      )
                    }
                  />
                ))}
            </Card>
          )}
          <Card>
            <Label>Agents</Label>
            {current.members.map((agent) => {
              const guest = policy.guest_access.find(
                (row) => row.agent_id === agent.id,
              );
              return (
                <View key={agent.id} style={{ gap: 8 }}>
                  <Label>
                    {agent.display_name ?? agent.name}
                    {agent.shared ? " · Shared" : ""}
                  </Label>
                  <Label muted>
                    {guest?.lane === "guest"
                      ? "Guest replies use the owner's services and bill the agent owner."
                      : agent.unavailable
                        ? "Ask this agent's owner to share it with you."
                        : "Your requests use your own access and billing rules."}
                  </Label>
                  <Label muted>
                    Guest replies: {guest?.enabled ? "On" : "Off"}
                  </Label>
                  {guest &&
                    (guest.enabled ? guest.can_disable : guest.can_enable) && (
                      <ActionButton
                        compact
                        label={
                          guest.enabled
                            ? "Disable guest replies"
                            : "Enable guest replies"
                        }
                        disabled={disabled}
                        onPress={() =>
                          guest.enabled
                            ? apply({
                                action: "guest_access",
                                agent_id: agent.id,
                                enabled: false,
                                guest_revision: guest.revision,
                              })
                            : confirm(
                                "Current and future participants without use rights can use your guest service policy. You pay. They cannot approve actions or use private account tools.",
                                {
                                  action: "guest_access",
                                  agent_id: agent.id,
                                  enabled: true,
                                  guest_revision: guest.revision,
                                },
                              )
                        }
                      />
                    )}
                  {policy.can_manage && (
                    <>
                      {current.lead_agent_id !== agent.id && (
                        <ActionButton
                          compact
                          label="Set default recipient"
                          disabled={disabled}
                          onPress={() =>
                            apply({ action: "lead", agent_id: agent.id })
                          }
                        />
                      )}
                      <ActionButton
                        compact
                        variant="danger"
                        label={`Remove agent ${agent.name}`}
                        disabled={disabled || current.members.length <= 1}
                        onPress={() =>
                          confirm(
                            `${agent.name} leaves this group and its queued work here is cancelled. The agent itself is not deleted.`,
                            { action: "remove_agent", agent_id: agent.id },
                          )
                        }
                      />
                    </>
                  )}
                </View>
              );
            })}
            <WorkspaceSelectionField
              label={
                policy.can_manage ? "Add specialist" : "Propose specialist"
              }
              title="Choose a specialist"
              value={[]}
              options={(agents.data?.agents ?? [])
                .filter(
                  (a) =>
                    a.kind === "specialist" &&
                    a.status !== "destroyed" &&
                    a.can_use === true &&
                    !current.members.some((m) => m.id === a.id),
                )
                .map((a) => ({
                  value: a.id,
                  label: a.display_name ?? a.name,
                  description: a.owner_name ?? undefined,
                }))}
              disabled={disabled || current.members.length >= 8}
              scopeKey={`${session.epoch}:${group.id}:new-agent`}
              onChange={([agent_id]) => {
                if (agent_id)
                  apply({
                    action: policy.can_manage ? "add_agent" : "propose_agent",
                    agent_id,
                  });
              }}
            />
            {policy.proposals.map((row) => (
              <View key={row.id}>
                <Label>Proposed agent {row.agent_id}</Label>
                {policy.can_manage ? (
                  <ActionButton
                    label="Accept proposed agent"
                    disabled={disabled}
                    onPress={() =>
                      apply({ action: "accept_agent", proposal_id: row.id })
                    }
                  />
                ) : (
                  <Label muted>Awaiting creator</Label>
                )}
              </View>
            ))}
          </Card>
          <ActionButton
            label="Done"
            variant="secondary"
            onPress={() => onSaved(group.id)}
          />
          <ActionButton
            label="Leave group"
            variant="danger"
            disabled={disabled}
            onPress={() =>
              confirm(
                policy.can_manage
                  ? "Transfer management first. Leaving as creator without an accepted transfer deletes this group for everyone."
                  : "You will lose access to group history. Your queued work is cancelled.",
                { action: "leave" },
              )
            }
          />
          {policy.can_manage && (
            <ActionButton
              label="Delete group for everyone"
              variant="danger"
              disabled={disabled}
              onPress={() =>
                confirm(
                  "Delete the group and transcript for everyone? This cannot be undone.",
                  { action: "delete" },
                )
              }
            />
          )}
        </>
      ) : detail.isError ? null : (
        <SkeletonRows
          rows={4}
          accessibilityLabel="Loading current group rights"
        />
      )}
    </WorkspacePage>
  );
}

export function CollaborationEntry({
  onOpenGroup,
}: {
  onOpenGroup: (id: string) => void;
}) {
  const session = useCollaborationSession("assistant:shared-agents", true);
  const [mode, setMode] = useState<"create" | "join">();
  const [name, setName] = useState("");
  const [members, setMembers] = useState<string[]>([]);
  const [token, setToken] = useState("");
  const [preview, setPreview] =
    useState<Awaited<ReturnType<typeof collaborationApi.preview>>>();
  const [disclosed, setDisclosed] = useState(false);
  useEffect(() => {
    if (!session.active) {
      setToken("");
      setPreview(undefined);
      setDisclosed(false);
    }
  }, [session.active, session.epoch]);
  useEffect(() => {
    setToken("");
    setPreview(undefined);
    setDisclosed(false);
  }, [session.epoch]);
  const agents = useQuery({
    queryKey: [...session.key, "agents"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: session.active && mode === "create",
    retry: false,
  });
  if (session.active && session.identity.isError)
    return (
      <Card>
        <Notice
          tone="error"
          message="Collaboration availability could not be verified."
        />
        <ActionButton
          label="Retry collaboration availability"
          onPress={() => void session.identity.refetch()}
        />
      </Card>
    );
  if (!session.enabled) return null;
  const available =
    session.capability.data === true &&
    !session.capability.isError &&
    session.capability.dataUpdatedAt >= session.activationAt;
  const disabled = !session.active || session.busy;
  return (
    <Card>
      <Label>Multi-person collaboration</Label>
      {session.error && <Notice tone="error" message={session.error} />}
      {!available && (
        <Label muted>
          New collaboration work is unavailable until the server rollout is
          ready. Existing groups and membership controls remain available.
        </Label>
      )}
      {!mode ? (
        <>
          <ActionButton
            label="Create multi-person group"
            disabled={disabled || !available}
            onPress={() => {
              if (disabled) return;
              session.clearError();
              setMode("create");
            }}
          />
          <ActionButton
            label="Join with invitation"
            variant="secondary"
            disabled={disabled || !available}
            onPress={() => {
              if (disabled) return;
              session.clearError();
              setMode("join");
            }}
          />
        </>
      ) : mode === "create" ? (
        <>
          <Field
            label="New group name"
            value={name}
            onChangeText={setName}
            maxLength={60}
            editable={!disabled}
          />
          <WorkspaceSelectionField
            label="Specialists"
            title="Choose group specialists"
            multiple
            maxSelected={8}
            value={members}
            options={(agents.data?.agents ?? [])
              .filter(
                (a) =>
                  a.kind === "specialist" &&
                  a.status !== "destroyed" &&
                  a.can_use === true,
              )
              .map((a) => ({
                value: a.id,
                label: a.display_name ?? a.name,
                description: a.owner_name ?? undefined,
              }))}
            disabled={disabled || agents.isError}
            scopeKey={`${session.epoch}:collaboration-create`}
            onChange={setMembers}
          />
          {agents.isError && (
            <Notice
              tone="error"
              message="Specialists could not load. Refresh the crew list and try again."
            />
          )}
          <ActionButton
            label="Create group"
            disabled={disabled || !available || !name.trim() || !members.length}
            onPress={() =>
              void session.run(
                () =>
                  groupsApi.create(
                    {
                      name: name.trim(),
                      member_agent_ids: members,
                      collaboration: true,
                    },
                    { authorityGuard: session.guard },
                  ),
                (group) => onOpenGroup(group.id),
              )
            }
          />
        </>
      ) : (
        <>
          <Field
            label="Invitation link or token"
            value={token}
            onChangeText={(value) => {
              session.clearError();
              setToken(value);
              setPreview(undefined);
              setDisclosed(false);
            }}
            editable={!disabled}
            autoCorrect={false}
          />
          {!preview ? (
            <ActionButton
              label="Preview invitation"
              disabled={disabled || !token.trim()}
              onPress={() =>
                void session.run(
                  () => collaborationApi.preview(inviteToken(token)),
                  (result) => setPreview(result),
                )
              }
            />
          ) : (
            <>
              <Label>
                {preview.group_name} · {preview.creator_name ?? "Group creator"}
              </Label>
              <Label muted>
                {preview.participants?.map((p) => p.display_name).join(" · ")}
              </Label>
              <Notice message={preview.history_disclosure} />
              <Label muted>
                Expires {new Date(preview.expires_at).toLocaleString()}
              </Label>
              <ActionButton
                label={
                  disclosed
                    ? "History disclosure acknowledged"
                    : "I understand the history disclosure"
                }
                variant="secondary"
                disabled={disabled}
                onPress={() => setDisclosed((value) => !value)}
              />
              <ActionButton
                label="Join group"
                disabled={
                  disabled ||
                  !available ||
                  !disclosed ||
                  Date.parse(preview.expires_at) <= Date.now()
                }
                onPress={() =>
                  void session.run(
                    () =>
                      collaborationApi.accept(token, {
                        authorityGuard: session.guard,
                      }),
                    (group) => onOpenGroup(group.id),
                  )
                }
              />
            </>
          )}
        </>
      )}
      {mode && (
        <ActionButton
          label="Cancel"
          variant="secondary"
          disabled={session.busy}
          onPress={() => {
            if (session.busy) return;
            session.clearError();
            setMode(undefined);
            setPreview(undefined);
            setToken("");
            setMembers([]);
            setDisclosed(false);
          }}
        />
      )}
    </Card>
  );
}

export function CollaborationCards({
  group,
  decisions,
}: {
  group: Group;
  decisions: readonly CollaborationDecision[];
}) {
  const session = useCollaborationSession();
  if (!session.readable) return null;
  return (
    <>
      {session.error && <Notice tone="error" message={session.error} />}
      {decisions.map((card, index) => (
        <Card key={card.id ?? `${card.requester_id}:${index}`}>
          <Label>
            {card.can_decide
              ? card.summary
              : `Waiting for ${group.participants?.find((p) => p.id === card.requester_id)?.display_name ?? "participant"}`}
          </Label>
          {card.can_decide &&
            card.requester_id === session.identity.data?.id &&
            card.id &&
            card.kind && (
              <>
                <Label muted>
                  {card.audience ?? "Everyone in this group can see the reply."}
                </Label>
                {card.connection_id && (
                  <Label>
                    Your connection:{" "}
                    {card.connection_name ?? card.connection_id}
                  </Label>
                )}
                {(card.kind === "consent"
                  ? [
                      ["once", "Allow once"],
                      ["always", "Always"],
                      ["skip", "Skip"],
                    ]
                  : [
                      ["allow", "Allow"],
                      ["deny", "Deny"],
                    ]
                ).map(([decision, label]) => (
                  <ActionButton
                    key={decision}
                    compact
                    label={label!}
                    disabled={session.busy || !session.active}
                    onPress={() =>
                      void session.run(() =>
                        collaborationApi.decide(
                          group.id,
                          card.id!,
                          card.kind!,
                          decision!,
                          { authorityGuard: session.guard },
                        ),
                      )
                    }
                  />
                ))}
              </>
            )}
        </Card>
      ))}
      {group.collaboration?.your_request_ids.map((id) => (
        <ActionButton
          key={id}
          compact
          variant="danger"
          label="Stop my group request"
          disabled={session.busy || !session.active}
          onPress={() =>
            Alert.alert(
              "Stop your group request?",
              "Queued and running work for this request is cancelled.",
              [
                { text: "Keep running", style: "cancel" },
                {
                  text: "Stop request",
                  style: "destructive",
                  onPress: () =>
                    void session.run(() =>
                      collaborationApi.cancel(group.id, id, {
                        authorityGuard: session.guard,
                      }),
                    ),
                },
              ],
            )
          }
        />
      ))}
    </>
  );
}

/** A participant or creator asks the agent's owner to allow participants. */
export function GroupAgentAccessRequest({
  group,
  member,
}: {
  group: Group;
  member: GroupMember;
}) {
  const session = useCollaborationSession();
  const policy = group.collaboration?.guest_access.find(
    (row) => row.agent_id === member.id,
  );
  if (!policy || !session.readable) return null;
  const request = policy.request;
  const disabled = session.busy || !session.active;
  const owner = member.owner_name ?? "owner";
  return (
    <View style={{ gap: 6 }}>
      {session.error && <Notice tone="error" message={session.error} />}
      {request?.status === "pending" ? (
        <>
          <Label muted>
            Owner approval pending · expires{" "}
            {new Date(request.expires_at).toLocaleDateString()}
          </Label>
          {request.can_cancel && (
            <ActionButton
              compact
              variant="secondary"
              label={`Cancel access request for ${member.name}`}
              disabled={disabled}
              onPress={() =>
                void session.run(() =>
                  collaborationApi.cancelGuestRequest(
                    group.id,
                    member.id,
                    request.id,
                    { authorityGuard: session.guard },
                  ),
                )
              }
            />
          )}
        </>
      ) : policy.can_request ? (
        <>
          {(request?.status === "expired" ||
            request?.status === "cancelled") && (
            <Label muted>Previous request {request.status}.</Label>
          )}
          <ActionButton
            compact
            label={`Ask ${owner} to allow participants`}
            disabled={disabled}
            onPress={() =>
              void session.run(() =>
                collaborationApi.askOwner(group.id, member.id, {
                  authorityGuard: session.guard,
                }),
              )
            }
          />
        </>
      ) : null}
    </View>
  );
}

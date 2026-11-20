import { discardUploadDraft } from "../assistant/assistantDraftStore";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import { localizedError } from "../../i18n/localizedError";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
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
  useAppLanguage();
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
        if (!policy) throw localizedError("Reload group settings first.");
        const fresh = await groupsApi.detail(group.id);
        if (!session.valid())
          throw localizedError("Keep group settings open to save.");
        if (
          fresh.collaboration?.membership_revision !==
          policy.membership_revision
        )
          throw localizedError("This group changed elsewhere. Refresh and review its current participants before saving.");
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
        ) {
          void discardUploadDraft(getAuthAccountId(), `crew:${group.id}`).catch(() => {});
          onRemoved();
        }
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
    Alert.alert(translateApp("Confirm group change"), message, [
      { text: translateApp("Cancel"), style: "cancel" },
      {
        text: translateApp("Confirm"),
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
      <WorkspacePage title={translateApp("Group settings")}>
        <Label muted>
          {translateApp("Return to this view to reload current group rights.")}
        </Label>
      </WorkspacePage>
    );
  return (
    <WorkspacePage
      title={translateApp("Group participants and agents")}
      onRefresh={pull.onRefresh}
      refreshing={pull.refreshing}
    >
      {session.error && <Notice tone="error" message={session.error} />}
      {detail.isError && (
        <Notice
          tone="error"
          message={translateApp(
            "Group access changed or settings could not load. Refresh to review. Actions are disabled.",
          )}
        />
      )}
      {policy && current ? (
        <>
          <Notice message={policy.history_disclosure} />
          {policy.can_manage && (
            <Card>
              <Field
                label={translateApp("Group name")}
                value={name}
                onChangeText={setName}
                editable={!disabled}
                maxLength={60}
              />
              <ActionButton
                label={translateApp("Save group name")}
                disabled={disabled || !name.trim() || name === current.name}
                onPress={() => apply({ action: "rename", name: name.trim() })}
              />
            </Card>
          )}
          <Card>
            <Label>{translateApp("Participants ({{count}} of 16)", { count: current.participants?.length ?? 0})}</Label>
            {current.participants?.map((person) => (
              <View key={person.id} style={{ gap: 8 }}>
                <Label>
                  {(person.id === policy.creator_user_id ? translateApp("{{name}} · Creator", { name: person.display_name }) : person.display_name)}
                </Label>
                {policy.can_manage && person.id !== policy.creator_user_id && (
                  <>
                    <ActionButton
                      compact
                      label={translateApp("Transfer management to {{name}}", {
                        name: person.display_name,
                      })}
                      disabled={disabled}
                      onPress={() =>
                        confirm(
                          translateApp(
                            "They must accept management before you can leave. This does not transfer any agent or credentials.",
                          ),
                          { action: "propose_transfer", person_id: person.id },
                        )
                      }
                    />
                    <ActionButton
                      compact
                      variant="danger"
                      label={translateApp("Remove {{name}}", {
                        name: person.display_name,
                      })}
                      disabled={disabled}
                      onPress={() =>
                        confirm(
                          translateApp(
                            "Their group access ends immediately and queued work is cancelled.",
                          ),
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
                label={translateApp("Accept group management")}
                disabled={disabled}
                onPress={() =>
                  confirm(
                    translateApp(
                      "You will manage group participants and invitations. Agent ownership stays unchanged.",
                    ),
                    { action: "accept_transfer" },
                  )
                }
              />
            ) : policy.transfer_to_user_id ? (
              <Label muted>
                {translateApp(
                  "Waiting for the proposed creator to accept management.",
                )}
              </Label>
            ) : null}
          </Card>
          {policy.can_manage && (
            <Card>
              <ActionButton
                label={translateApp("Create invitation link")}
                disabled={disabled || (current.participants?.length ?? 0) >= 16}
                onPress={() => apply({ action: "invite" })}
              />
              <Label muted>
                {translateApp(
                  "Single use, expires in seven days. Share it yourself. Joining exposes the full retained group history.",
                )}
              </Label>
              {invite && (
                <>
                  <Label selectable>
                    {invitationLink(getFrontendUrl(), invite.token) ??
                      invite.token}
                  </Label>
                  <Label muted>{translateApp("Expires {{time}}", { time: new Date(invite.expires_at).toLocaleString(appLocale()) })}
                  </Label>
                  <ActionButton
                    label={
                      invitationLink(getFrontendUrl(), invite.token)
                        ? translateApp("Copy invitation link")
                        : translateApp("Copy invitation token")
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
                  label={translateApp("Revoke invitation · {{date}}", {
                    date: new Date(row.expires_at).toLocaleDateString(
                      appLocale(),
                    ),
                  })}
                  disabled={disabled}
                  onPress={() =>
                    apply({ action: "cancel_invite", invite_id: row.id })
                  }
                />
              ))}
              <WorkspaceSelectionField
                label={translateApp("Quick-add from organization")}
                title={translateApp("Choose a shared organization")}
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
                  message={translateApp("Organization members could not load.")}
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
                    label={translateApp("Add {{email}}", {
                      email: person.display_name ?? person.email ?? "member",
                    })}
                    disabled={
                      disabled || (current.participants?.length ?? 0) >= 16
                    }
                    onPress={() =>
                      confirm(
                        translateApp(
                          "This person will immediately see the full retained group history.",
                        ),
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
            <Label>{translateApp("Agents")}</Label>
            {current.members.map((agent) => {
              const guest = policy.guest_access.find(
                (row) => row.agent_id === agent.id,
              );
              return (
                <View key={agent.id} style={{ gap: 8 }}>
                  <Label>
                    {(agent.shared ? translateApp("{{agent}} · Shared", { agent: agent.display_name ?? agent.name}) : agent.display_name ?? agent.name)}
                  </Label>
                  <Label muted>
                    {guest?.lane === "guest"
                      ? translateApp(
                          "Guest replies use the owner's services and bill the agent owner.",
                        )
                      : agent.unavailable
                        ? translateApp(
                            "Ask this agent's owner to share it with you.",
                          )
                        : translateApp(
                            "Your requests use your own access and billing rules.",
                          )}
                  </Label>
                  <Label muted>{(guest?.enabled ? translateApp("Guest replies: On") : translateApp("Guest replies: Off"))}
                  </Label>
                  {guest &&
                    (guest.enabled ? guest.can_disable : guest.can_enable) && (
                      <ActionButton
                        compact
                        label={
                          guest.enabled
                            ? translateApp("Disable guest replies")
                            : translateApp("Enable guest replies")
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
                                translateApp(
                                  "Current and future participants without use rights can use your guest service policy. You pay. They cannot approve actions or use private account tools.",
                                ),
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
                          label={translateApp("Set default recipient")}
                          disabled={disabled}
                          onPress={() =>
                            apply({ action: "lead", agent_id: agent.id })
                          }
                        />
                      )}
                      <ActionButton
                        compact
                        variant="danger"
                        label={translateApp("Remove agent {{name}}", {
                          name: agent.name,
                        })}
                        disabled={disabled || current.members.length <= 1}
                        onPress={() =>
                          confirm(
                            translateApp(
                              "{{name}} leaves this group and its queued work here is cancelled. The agent itself is not deleted.",
                              { name: agent.name },
                            ),
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
                policy.can_manage ? translateApp("Add specialist")
                  : translateApp("Propose specialist")
              }
              title={translateApp("Choose a specialist")}
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
                <Label>{translateApp("Proposed agent {{agentId}}", { agentId: row.agent_id })}</Label>
                {policy.can_manage ? (
                  <ActionButton
                    label={translateApp("Accept proposed agent")}
                    disabled={disabled}
                    onPress={() =>
                      apply({ action: "accept_agent", proposal_id: row.id })
                    }
                  />
                ) : (
                  <Label muted>{translateApp("Awaiting creator")}</Label>
                )}
              </View>
            ))}
          </Card>
          <ActionButton
            label={translateApp("Done")}
            variant="secondary"
            onPress={() => onSaved(group.id)}
          />
          <ActionButton
            label={translateApp("Leave group")}
            variant="danger"
            disabled={disabled}
            onPress={() =>
              confirm(
                policy.can_manage
                  ? translateApp(
                      "Transfer management first. Leaving as creator without an accepted transfer deletes this group for everyone.",
                    )
                  : translateApp(
                      "You will lose access to group history. Your queued work is cancelled.",
                    ),
                { action: "leave" },
              )
            }
          />
          {policy.can_manage && (
            <ActionButton
              label={translateApp("Delete group for everyone")}
              variant="danger"
              disabled={disabled}
              onPress={() =>
                confirm(
                  translateApp(
                    "Delete the group and transcript for everyone? This cannot be undone.",
                  ),
                  { action: "delete" },
                )
              }
            />
          )}
        </>
      ) : detail.isError ? null : (
        <SkeletonRows
          rows={4}
          accessibilityLabel={translateApp("Loading current group rights")}
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
  useAppLanguage();
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
          message={translateApp(
            "Collaboration availability could not be verified.",
          )}
        />
        <ActionButton
          label={translateApp("Retry collaboration availability")}
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
      <Label>{translateApp("Multi-person collaboration")}</Label>
      {session.error && <Notice tone="error" message={session.error} />}
      {!available && (
        <Label muted>
          {translateApp(
            "New collaboration work is unavailable until the server rollout is ready. Existing groups and membership controls remain available.",
          )}
        </Label>
      )}
      {!mode ? (
        <>
          <ActionButton
            label={translateApp("Create multi-person group")}
            disabled={disabled || !available}
            onPress={() => {
              if (disabled) return;
              session.clearError();
              setMode("create");
            }}
          />
          <ActionButton
            label={translateApp("Join with invitation")}
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
            label={translateApp("New group name")}
            value={name}
            onChangeText={setName}
            maxLength={60}
            editable={!disabled}
          />
          <WorkspaceSelectionField
            label={translateApp("Specialists")}
            title={translateApp("Choose group specialists")}
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
              message={translateApp(
                "Specialists could not load. Refresh the crew list and try again.",
              )}
            />
          )}
          <ActionButton
            label={translateApp("Create group")}
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
            label={translateApp("Invitation link or token")}
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
              label={translateApp("Preview invitation")}
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
                {translateApp("{{name}} · {{nameOther}}", { name: preview.group_name, nameOther: preview.creator_name ?? translateApp("Group creator") })}
              </Label>
              <Label muted>
                {preview.participants?.map((p) => p.display_name).join(" · ")}
              </Label>
              <Notice message={preview.history_disclosure} />
              <Label muted>{translateApp("Expires {{time}}", { time: new Date(preview.expires_at).toLocaleString(appLocale()) })}
              </Label>
              <ActionButton
                label={
                  disclosed
                    ? translateApp("History disclosure acknowledged")
                    : translateApp("I understand the history disclosure")
                }
                variant="secondary"
                disabled={disabled}
                onPress={() => setDisclosed((value) => !value)}
              />
              <ActionButton
                label={translateApp("Join group")}
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
          label={translateApp("Cancel")}
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
  useAppLanguage();
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
              : translateApp("Waiting for {{details}}", {
                  details:
                    group.participants?.find((p) => p.id === card.requester_id)?.display_name ?? "participant",
                })}
          </Label>
          {card.can_decide &&
            card.requester_id === session.identity.data?.id &&
            card.id &&
            card.kind && (
              <>
                <Label muted>
                  {card.audience ??
                    translateApp("Everyone in this group can see the reply.")}
                </Label>
                {card.connection_id && (
                  <Label>{translateApp("Your connection: {{name}}", { name: card.connection_name ?? card.connection_id })}
                  </Label>
                )}
                {(card.kind === "consent"
                  ? [
                      ["once", translateApp("Allow once")],
                      ["always", translateApp("Always")],
                      ["skip", translateApp("Skip")],
                    ]
                  : [
                      ["allow", translateApp("Allow")],
                      ["deny", translateApp("Deny")],
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
          label={translateApp("Stop my group request")}
          disabled={session.busy || !session.active}
          onPress={() =>
            Alert.alert(
              translateApp("Stop your group request?"),
              translateApp(
                "Queued and running work for this request is cancelled.",
              ),
              [
                { text: translateApp("Keep running"), style: "cancel" },
                {
                  text: translateApp("Stop request"),
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
  useAppLanguage();
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
          <Label muted>{translateApp("Owner approval pending · expires {{time}}", { time: new Date(request.expires_at).toLocaleDateString(appLocale()) })}
          </Label>
          {request.can_cancel && (
            <ActionButton
              compact
              variant="secondary"
              label={translateApp("Cancel access request for {{name}}", {
                name: member.name,
              })}
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
            <Label muted>{translateApp("Previous request {{status}}.", { status: request.status})}</Label>
          )}
          <ActionButton
            compact
            label={translateApp("Ask {{organization}} to allow participants", {
              organization: owner,
            })}
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

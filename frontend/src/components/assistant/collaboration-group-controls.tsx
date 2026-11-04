import { flushSync } from "react-dom";
import { useState } from "react";
import { Switch } from "@/components/ui/switch";
import { GroupAgentAccessRequest } from "./group-agent-access";
import { groupAgentUnavailableReason } from "@/lib/assistant/nyxbot-labels";
import { useQueryClient } from "@tanstack/react-query";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import {
  GroupedList,
  GroupedRow,
  GroupedInput as Input,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedField,
  GroupedLabel,
  GroupedMessage,
} from "@/components/ui/grouped-fields";
import {
  GroupedFormActions,
  GroupedFormScope,
  GroupedFormBar,
} from "@/components/ui/grouped-form";
import {
  GroupedDialog,
  GroupedDialogContent,
  GroupedDialogCancel,
  GroupedDecisionDraft,
} from "@/components/ui/grouped-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { Form, FormField, FormControl, useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import {
  useCollaborationChange,
  useCollaborationDecision,
  collaborationApi,
} from "@/hooks/use-collaboration-groups";
import { useOrgs } from "@/hooks/use-orgs";
import { useOrgMembers } from "@/hooks/use-org-members";
import { nyxBotGroupKeys } from "@/hooks/use-nyxbot-groups";
import { useAuthStore } from "@/stores/auth-store";
import type {
  AssistantAgent,
  AssistantGroup,
} from "@/schemas/assistant-nyxagent";
import type {
  CollaborationChange,
  CollaborationDecision,
} from "@/schemas/collaboration-group";
import { agentTitle } from "@/lib/assistant/nyxbot-labels";

const errorText = (error: unknown) =>
  error instanceof Error
    ? error.message
    : "The change could not be saved. Reload and try again.";
const renameSchema = z.object({ name: z.string().trim().min(1).max(60) });

function QuickAdd({
  group,
  busy,
  onAdd,
}: {
  readonly group: AssistantGroup;
  readonly busy: boolean;
  readonly onAdd: (person: string) => void;
}) {
  const orgs = useOrgs();
  const [org, setOrg] = useState("");
  const members = useOrgMembers(org);
  return (
    <GroupedList
      title="Quick-add from your organization"
      footer="New participants can read the full retained group history."
    >
      <GroupedRow
        title="Organization"
        accessory={
          <Select value={org} onValueChange={setOrg}>
            <SelectTrigger aria-label="Shared organization">
              <SelectValue placeholder="Choose organization" />
            </SelectTrigger>
            <SelectContent>
              {orgs.data?.map((row) => (
                <SelectItem key={row.id} value={row.id}>
                  {row.display_name ?? row.slug}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
      />
      {(members.data ?? [])
        .filter(
          (p) =>
            !p.revoked_at &&
            !group.participants?.some((member) => member.id === p.user_id),
        )
        .map((person) => (
          <GroupedRow
            key={person.user_id}
            title={person.display_name ?? "Member"}
            accessory={
              <Button
                variant="ghost"
                size="sm"
                disabled={busy || (group.participants?.length ?? 0) >= 16}
                onClick={() => onAdd(person.user_id)}
              >
                Add participant
              </Button>
            }
          />
        ))}
    </GroupedList>
  );
}

function Rename({
  name,
  save,
  busy,
}: {
  readonly name: string;
  readonly save: (name: string) => Promise<boolean>;
  readonly busy: boolean;
}) {
  const form = useAppForm<z.infer<typeof renameSchema>>({
    resolver: zodResolver(renameSchema),
    defaultValues: { name },
  });
  const submit = form.handleSubmit(async (values) => {
    if (await save(values.name)) form.reset(values);
  });
  return (
    <Form {...form}>
      <form onSubmit={submit} className="space-y-6">
        <GroupedList title="Profile">
          <FormField
            control={form.control}
            name="name"
            render={({ field }) => (
              <GroupedField>
                <GroupedLabel>Group name</GroupedLabel>
                <FormControl>
                  <Input {...field} maxLength={60} />
                </FormControl>
                <GroupedMessage />
              </GroupedField>
            )}
          />
        </GroupedList>
        <GroupedFormActions
          label="group name"
          dirty={form.formState.isDirty}
          pending={busy}
          save={submit}
          discard={() => form.reset()}
          saveLabel="Save"
        />
      </form>
    </Form>
  );
}

export function CollaborationSettings({
  group,
  agents,
  onClose,
  onLeft,
}: {
  readonly group: AssistantGroup;
  readonly agents: readonly AssistantAgent[];
  readonly onClose: () => void;
  readonly onLeft: () => void;
}) {
  const policy = group.collaboration!;
  const actor = useAuthStore((s) => s.user?.id);
  const mutation = useCollaborationChange(group.id, policy.membership_revision);
  const [error, setError] = useState<string>();
  const [invite, setInvite] = useState<{ token: string; expires_at: string }>();
  const [confirm, setConfirm] = useState<{
    text: string;
    change: CollaborationChange;
  }>();
  async function apply(change: CollaborationChange) {
    setError(undefined);
    try {
      const result = await mutation.mutateAsync(change);
      if (result.invite) setInvite(result.invite);
      if (!result.group && ["leave", "delete"].includes(change.action))
        onLeft();
      return true;
    } catch (cause) {
      setError(errorText(cause));
      return false;
    }
  }
  const eligible = agents.filter(
    (a) =>
      a.kind === "specialist" &&
      a.status !== "destroyed" &&
      a.can_use !== false &&
      !group.members.some((m) => m.id === a.id),
  );
  return (
    <GroupedDialog
      open
      onOpenChange={(open) => {
        if (!open && !mutation.isPending) onClose();
      }}
    >
      <GroupedDialogContent scrollMode="body" className="md:max-w-lg">
        <DialogHeader>
          <DialogTitle>Group participants and agents</DialogTitle>
          <DialogDescription>{policy.history_disclosure}</DialogDescription>
        </DialogHeader>
        <GroupedFormScope>
          <DialogBody className="space-y-6 pb-4">
            {policy.can_manage && (
              <Rename
                name={group.name}
                busy={mutation.isPending}
                save={(name) => apply({ action: "rename", name })}
              />
            )}
            <GroupedList
              title={`Participants (${group.participants?.length ?? 0} of 16)`}
              aria-label="Group participants"
              footer={
                policy.transfer_to_user_id &&
                policy.transfer_to_user_id !== actor
                  ? "Waiting for the proposed creator to accept management."
                  : undefined
              }
            >
              {group.participants?.map((person) => (
                <GroupedRow
                  key={person.id}
                  title={`${person.display_name}${person.id === actor ? " (you)" : ""}`}
                  accessory={
                    person.id === policy.creator_user_id ? (
                      <span className="text-11 text-text-tertiary">
                        Creator
                      </span>
                    ) : policy.can_manage ? (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={mutation.isPending}
                          onClick={() =>
                            setConfirm({
                              text: `Offer group management to ${person.display_name}? They must accept before you can leave.`,
                              change: {
                                action: "propose_transfer",
                                person_id: person.id,
                              },
                            })
                          }
                        >
                          Transfer
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive"
                          disabled={mutation.isPending}
                          onClick={() =>
                            setConfirm({
                              text: `Remove ${person.display_name} from this group? Their queued work is cancelled and access ends immediately.`,
                              change: {
                                action: "remove",
                                person_id: person.id,
                              },
                            })
                          }
                        >
                          Remove
                        </Button>
                      </>
                    ) : undefined
                  }
                />
              ))}
              {policy.transfer_to_user_id === actor && (
                <GroupedRow asChild interactive>
                  <button
                    type="button"
                    disabled={mutation.isPending}
                    onClick={() =>
                      setConfirm({
                        text: "Accept management of this group? You will manage its participants and invitations.",
                        change: { action: "accept_transfer" },
                      })
                    }
                  >
                    Accept management
                  </button>
                </GroupedRow>
              )}
            </GroupedList>
            {policy.can_manage && (
              <>
                <GroupedList
                  title="Invitations"
                  aria-label="Group invitations"
                  footer={
                    <>
                      Single use, expires after seven days. A NyxID account is
                      required. Send the link yourself.
                      {invite && (
                        <p>
                          Expires {new Date(invite.expires_at).toLocaleString()}
                        </p>
                      )}
                    </>
                  }
                >
                  <GroupedRow asChild interactive>
                    <button
                      type="button"
                      disabled={
                        mutation.isPending ||
                        (group.participants?.length ?? 0) >= 16
                      }
                      onClick={() => void apply({ action: "invite" })}
                    >
                      Create invitation link
                    </button>
                  </GroupedRow>
                  {invite && (
                    <>
                      <GroupedRow>
                        <Input
                          aria-label="Invitation link"
                          readOnly
                          value={`${window.location.origin}/assistant#group-invite=${invite.token}`}
                        />
                      </GroupedRow>
                      <GroupedRow asChild interactive>
                        <button
                          type="button"
                          onClick={() =>
                            void navigator.clipboard.writeText(
                              `${window.location.origin}/assistant#group-invite=${invite.token}`,
                            )
                          }
                        >
                          Copy link
                        </button>
                      </GroupedRow>
                    </>
                  )}
                  {policy.invites.map((item) => (
                    <GroupedRow
                      key={item.id}
                      title="Invitation"
                      description={`Expires ${new Date(item.expires_at).toLocaleDateString()}`}
                      accessory={
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={mutation.isPending}
                          onClick={() =>
                            void apply({
                              action: "cancel_invite",
                              invite_id: item.id,
                            })
                          }
                        >
                          Cancel invitation
                        </Button>
                      }
                    />
                  ))}
                </GroupedList>
                <QuickAdd
                  group={group}
                  busy={mutation.isPending}
                  onAdd={(person_id) =>
                    setConfirm({
                      text: "This person will immediately see the full retained group history. Add them now?",
                      change: {
                        action: "quick_add",
                        person_id,
                        history_disclosed: true,
                      },
                    })
                  }
                />
              </>
            )}
            {group.members.map((agent) => {
              const guest = policy.guest_access.find(
                (g) => g.agent_id === agent.id,
              );
              return (
                <GroupedList
                  key={agent.id}
                  title={agentTitle(agent)}
                  footer={
                    guest?.lane === "guest"
                      ? "Your requests use guest access and bill the agent owner."
                      : agent.unavailable
                        ? groupAgentUnavailableReason(agent)
                        : "Your requests use your access and billing rules."
                  }
                >
                  <GroupedRow
                    title={agentTitle(agent)}
                    description={[
                      agent.shared ? "Shared agent" : null,
                      agent.owner_name ? `Owner: ${agent.owner_name}` : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  />
                  {guest?.can_enable ? (
                    <GroupedRow
                      title="Participants can use this agent"
                      accessory={
                        <Switch
                          aria-label={`Participants can use ${agentTitle(agent)}`}
                          checked={guest.enabled}
                          disabled={mutation.isPending}
                          onCheckedChange={(enabled) => enabled
                            ? setConfirm({
                                text: "Allow current and future participants without use rights to talk to this agent as guests? They use your guest service policy and you pay. They cannot approve actions or use your private account tools.",
                                change: { action: "guest_access", agent_id: agent.id, enabled: true, guest_revision: guest.revision },
                              })
                            : void apply({ action: "guest_access", agent_id: agent.id, enabled: false, guest_revision: guest.revision })}
                        />
                      }
                    />
                  ) : guest?.enabled && guest.can_disable ? (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={mutation.isPending}
                        onClick={() => void apply({ action: "guest_access", agent_id: agent.id, enabled: false, guest_revision: guest.revision })
                        }
                      >
                        Disable participant access
                      </button>
                    </GroupedRow>
                  ) : null}
                  <GroupAgentAccessRequest
                    grouped
                    group={group}
                    member={agent}
                  />
                  {policy.can_manage && group.lead_agent_id !== agent.id && (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={mutation.isPending}
                        onClick={() =>
                          void apply({ action: "lead", agent_id: agent.id })
                        }
                      >
                        Set as default recipient
                      </button>
                    </GroupedRow>
                  )}
                  {policy.can_manage && (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        className="text-destructive"
                        disabled={
                          mutation.isPending || group.members.length <= 1
                        }
                        onClick={() =>
                          void apply({
                            action: "remove_agent",
                            agent_id: agent.id,
                          })
                        }
                      >
                        Remove agent
                      </button>
                    </GroupedRow>
                  )}
                </GroupedList>
              );
            })}
            {(eligible.length > 0 || policy.proposals.length > 0) && (
              <GroupedList title="More agents" aria-label="Group agents">
                {eligible.length > 0 && (
                  <GroupedRow
                    title={
                      policy.can_manage
                        ? "Add a specialist"
                        : "Propose a specialist"
                    }
                    accessory={
                      <Select
                        onValueChange={(agent_id) =>
                          void apply({
                            action: policy.can_manage ? "add_agent" : "propose_agent",
                            agent_id,
                          })
                        }
                      >
                        <SelectTrigger
                          aria-label="Add specialist"
                          disabled={
                            mutation.isPending || group.members.length >= 8
                          }
                        >
                          <SelectValue placeholder="Choose specialist" />
                        </SelectTrigger>
                        <SelectContent>
                          {eligible.map((agent) => (
                            <SelectItem key={agent.id} value={agent.id}>
                              {agentTitle(agent)}
                              {agent.owner_name ? ` · ${agent.owner_name}` : ""}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    }
                  />
                )}
                {policy.proposals.map((proposal) => (
                  <GroupedRow
                    key={proposal.id}
                    title={`Proposed agent ${proposal.agent_id}`}
                    accessory={
                      policy.can_manage ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={mutation.isPending}
                          onClick={() =>
                            void apply({
                              action: "accept_agent",
                              proposal_id: proposal.id,
                            })
                          }
                        >
                          Accept agent
                        </Button>
                      ) : (
                        <span>Awaiting creator</span>
                      )
                    }
                  />
                ))}
              </GroupedList>
            )}
            <GroupedList
              title="Group actions"
              footer={
                <>
                  <p>
                    Only the person who requested work can decide its cards or
                    stop it. The creator must transfer management to an
                    accepting participant before leaving, or delete the group.
                  </p>
                  {error && (
                    <p role="alert" className="text-destructive">
                      {error}
                    </p>
                  )}
                </>
              }
            >
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  className="text-destructive"
                  disabled={mutation.isPending}
                  onClick={() =>
                    setConfirm({
                      text: "Leave this group? You will lose access to its history and your queued work will be cancelled.",
                      change: { action: "leave" },
                    })
                  }
                >
                  Leave group
                </button>
              </GroupedRow>
              {policy.can_manage && (
                <GroupedRow asChild interactive>
                  <button
                    type="button"
                    className="text-destructive"
                    disabled={mutation.isPending}
                    onClick={() =>
                      setConfirm({
                        text: "Delete this group and its transcript for everyone? This cannot be undone.",
                        change: { action: "delete" },
                      })
                    }
                  >
                    Delete group
                  </button>
                </GroupedRow>
              )}
            </GroupedList>
          </DialogBody>
          <GroupedFormBar />
        </GroupedFormScope>
        {confirm ? (
          <Dialog
            open
            onOpenChange={(open) => {
              if (!open) setConfirm(undefined);
            }}
          >
            <DialogContent className="md:max-w-md">
              <DialogHeader>
                <DialogTitle>Confirm group change</DialogTitle>
                <DialogDescription>{confirm.text}</DialogDescription>
              </DialogHeader>
              {error ? (
                <p role="alert" className="text-12 text-destructive">
                  {error}
                </p>
              ) : null}
              <DialogFooter>
                <Button onClick={() => setConfirm(undefined)}>Cancel</Button>
                <Button
                  variant="primary"
                  isLoading={mutation.isPending}
                  onClick={async () => {
                    if (await apply(confirm.change)) setConfirm(undefined);
                  }}
                >
                  Confirm
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        ) : null}
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

export function CollaborationCards({
  group,
  decisions,
}: {
  readonly group: AssistantGroup;
  readonly decisions: readonly CollaborationDecision[];
}) {
  const mutation = useCollaborationDecision(group.id);
  return (
    <div className="mx-auto max-w-3xl space-y-2 px-4">
      {decisions.map((card, i) => (
        <div
          key={card.id ?? `${card.requester_id}-${String(i)}`}
          className="space-y-2 rounded-lg border border-border bg-card p-3 text-12"
        >
          <p>
            {card.can_decide
              ? card.summary
              : `Waiting for ${group.participants?.find((p) => p.id === card.requester_id)?.display_name ?? "participant"}`}
          </p>
          {card.can_decide ? (
            <>
              <p className="text-muted-foreground">{card.audience}</p>
              {card.connection_id ? (
                <p>
                  Your connection: {card.connection_name ?? card.connection_id}
                </p>
              ) : null}
              <div className="flex gap-2">
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
                  <Button
                    key={decision}
                    size="sm"
                    isLoading={mutation.isPending}
                    onClick={() =>
                      mutation.mutate({
                        id: card.id!,
                        kind: card.kind!,
                        decision: decision!,
                      })
                    }
                  >
                    {label}
                  </Button>
                ))}
              </div>
            </>
          ) : null}
        </div>
      ))}
      {mutation.error ? (
        <p role="alert" className="text-12 text-destructive">
          {errorText(mutation.error)}
        </p>
      ) : null}
    </div>
  );
}

export function GroupInviteDialog({
  token,
  onClose,
  onJoined,
}: {
  readonly token: string;
  readonly onClose: () => void;
  readonly onJoined: (group: AssistantGroup) => void;
}) {
  const [preview, setPreview] =
    useState<Awaited<ReturnType<typeof collaborationApi.preview>>>();
  const [disclosed, setDisclosed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const client = useQueryClient();
  const actor = useAuthStore((s) => s.user?.id);
  async function proceed() {
    setBusy(true);
    setError(undefined);
    try {
      if (!preview) setPreview(await collaborationApi.preview(token));
      else {
        const group = await collaborationApi.accept(token);
        await client.invalidateQueries({
          queryKey: nyxBotGroupKeys.all(actor),
        });
        flushSync(() => setDisclosed(false));
        onJoined(group);
      }
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <GroupedDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <GroupedDialogContent className="md:max-w-md">
        <GroupedDecisionDraft
          dirty={disclosed}
          pending={busy}
          discard={() => setDisclosed(false)}
        />
        <DialogHeader>
          <DialogTitle>
            {preview ? `Join ${preview.group_name}` : "Group invitation"}
          </DialogTitle>
          <DialogDescription>
            {preview?.history_disclosure ??
              "Preview this invitation before joining. A NyxID account is required."}
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-3">
          {preview ? (
            <GroupedList
              title="Invitation"
              footer={`Expires ${new Date(preview.expires_at).toLocaleString()}`}
            >
              <GroupedRow
                title="Created by"
                accessory={preview.creator_name ?? "a NyxID user"}
              />
              <GroupedRow
                title="Participants"
                description={preview.participants
                  ?.map((p) => p.display_name)
                  .join(", ")}
              />
              <GroupedRow asChild interactive>
                <label>
                  <Checkbox
                    checked={disclosed}
                    onCheckedChange={(checked) => setDisclosed(checked === true)}
                  />
                  I understand everyone can read the full retained group
                  history.
                </label>
              </GroupedRow>
            </GroupedList>
          ) : null}
          {error ? (
            <GroupedList
              footer={
                <p role="alert" className="text-destructive">
                  {error}
                </p>
              }
            />
          ) : null}
        </DialogBody>
        <DialogFooter>
          <GroupedDialogCancel disabled={busy}>Cancel</GroupedDialogCancel>
          <Button
            variant="primary"
            isLoading={busy}
            disabled={Boolean(preview && !disclosed)}
            onClick={() => void proceed()}
          >
            {preview ? "Accept invitation" : "Preview invitation"}
          </Button>
        </DialogFooter>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

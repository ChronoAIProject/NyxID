import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import {
  Select,
  SelectTrigger,
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
    <div className="space-y-2">
      <p className="text-12 font-medium">Quick-add from your organization</p>
      <p className="text-12 text-muted-foreground">
        New participants can read the full retained group history.
      </p>
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
      {(members.data ?? [])
        .filter(
          (p) =>
            !p.revoked_at &&
            !group.participants?.some((member) => member.id === p.user_id),
        )
        .map((person) => (
          <div
            key={person.user_id}
            className="flex items-center justify-between gap-2 text-12"
          >
            <span>{person.display_name ?? "Member"}</span>
            <Button
              size="sm"
              disabled={busy || (group.participants?.length ?? 0) >= 16}
              onClick={() => onAdd(person.user_id)}
            >
              Add participant
            </Button>
          </div>
        ))}
    </div>
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
  return (
    <Form {...form}>
      <form
        className="flex items-end gap-2"
        onSubmit={form.handleSubmit(async (values) => {
          if (await save(values.name)) form.reset(values);
        })}
      >
        <FormField
          control={form.control}
          name="name"
          render={({ field }) => (
            <FormItem className="flex-1">
              <FormLabel>Group name</FormLabel>
              <FormControl>
                <Input {...field} maxLength={60} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <Button
          type="submit"
          variant="primary"
          isLoading={busy}
          disabled={!form.formState.isDirty}
        >
          Save
        </Button>
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
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !mutation.isPending) onClose();
      }}
    >
      <DialogContent scrollMode="body" className="md:max-w-lg">
        <DialogHeader>
          <DialogTitle>Group participants and agents</DialogTitle>
          <DialogDescription>{policy.history_disclosure}</DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-5">
          {policy.can_manage ? (
            <Rename
              name={group.name}
              busy={mutation.isPending}
              save={(name) => apply({ action: "rename", name })}
            />
          ) : null}
          <section className="space-y-2" aria-label="Group participants">
            <p className="text-12 font-medium">
              Participants ({group.participants?.length ?? 0} of 16)
            </p>
            {group.participants?.map((person) => (
              <div key={person.id} className="flex items-center gap-2 text-12">
                <span className="flex-1">
                  {person.display_name}
                  {person.id === actor ? " (you)" : ""}
                </span>
                {person.id === policy.creator_user_id ? (
                  <Badge variant="accent">Creator</Badge>
                ) : (
                  <>
                    {policy.can_manage ? (
                      <>
                        <Button
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
                          size="sm"
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
                    ) : null}
                  </>
                )}
              </div>
            ))}
            {policy.transfer_to_user_id === actor ? (
              <Button
                variant="primary"
                isLoading={mutation.isPending}
                onClick={() =>
                  setConfirm({
                    text: "Accept management of this group? You will manage its participants and invitations.",
                    change: { action: "accept_transfer" },
                  })
                }
              >
                Accept management
              </Button>
            ) : policy.transfer_to_user_id ? (
              <p className="text-12 text-muted-foreground">
                Waiting for the proposed creator to accept management.
              </p>
            ) : null}
          </section>
          {policy.can_manage ? (
            <section className="space-y-3" aria-label="Group invitations">
              <Button
                disabled={
                  mutation.isPending || (group.participants?.length ?? 0) >= 16
                }
                onClick={() => void apply({ action: "invite" })}
              >
                Create invitation link
              </Button>
              <p className="text-12 text-muted-foreground">
                Single use, expires after seven days. A NyxID account is
                required. Send the link yourself.
              </p>
              {invite ? (
                <div className="space-y-2">
                  <Input
                    aria-label="Invitation link"
                    readOnly
                    value={`${window.location.origin}/assistant#group-invite=${invite.token}`}
                  />
                  <Button
                    onClick={() =>
                      void navigator.clipboard.writeText(
                        `${window.location.origin}/assistant#group-invite=${invite.token}`,
                      )
                    }
                  >
                    Copy link
                  </Button>
                  <p className="text-11 text-text-tertiary">
                    Expires {new Date(invite.expires_at).toLocaleString()}
                  </p>
                </div>
              ) : null}
              {policy.invites.map((item) => (
                <div
                  key={item.id}
                  className="flex items-center justify-between gap-2 text-12"
                >
                  <span>
                    Invitation · expires{" "}
                    {new Date(item.expires_at).toLocaleDateString()}
                  </span>
                  <Button
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
                </div>
              ))}
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
            </section>
          ) : null}
          <section className="space-y-3" aria-label="Group agents">
            <p className="text-12 font-medium">Agents</p>
            {group.members.map((agent) => {
              const guest = policy.guest_access.find(
                (g) => g.agent_id === agent.id,
              );
              return (
                <div
                  key={agent.id}
                  className="space-y-2 rounded-lg border border-border p-3 text-12"
                >
                  <div className="flex items-center gap-2">
                    <span className="flex-1 font-medium">
                      {agentTitle(agent)}
                      {agent.shared ? " · Shared agent" : ""}
                    </span>
                    {policy.can_manage ? (
                      <Button
                        size="sm"
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
                      </Button>
                    ) : null}
                  </div>
                  <p className="text-muted-foreground">
                    {guest?.lane === "guest"
                      ? "Your requests use guest access and bill the agent owner."
                      : agent.unavailable
                        ? "Ask this agent’s owner to share it with you."
                        : "Your requests use your access and billing rules."}
                  </p>
                  <p>Guest replies: {guest?.enabled ? "On" : "Off"}</p>
                  {guest &&
                  (guest.enabled ? guest.can_disable : guest.can_enable) ? (
                    <Button
                      size="sm"
                      disabled={mutation.isPending}
                      onClick={() =>
                        guest.enabled
                          ? void apply({
                              action: "guest_access",
                              agent_id: agent.id,
                              enabled: false,
                              guest_revision: guest.revision,
                            })
                          : setConfirm({
                              text: "Allow current and future participants without use rights to talk to this agent as guests? They use your guest service policy and you pay. They cannot approve actions or use your private account tools.",
                              change: {
                                action: "guest_access",
                                agent_id: agent.id,
                                enabled: true,
                                guest_revision: guest.revision,
                              },
                            })
                      }
                    >
                      {guest.enabled
                        ? "Disable guest replies"
                        : "Enable guest replies"}
                    </Button>
                  ) : null}
                  {policy.can_manage && group.lead_agent_id !== agent.id ? (
                    <Button
                      size="sm"
                      disabled={mutation.isPending}
                      onClick={() =>
                        void apply({ action: "lead", agent_id: agent.id })
                      }
                    >
                      Set as default recipient
                    </Button>
                  ) : null}
                </div>
              );
            })}
            {eligible.length ? (
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
                  disabled={mutation.isPending || group.members.length >= 8}
                >
                  <SelectValue
                    placeholder={
                      policy.can_manage
                        ? "Add a specialist"
                        : "Propose a specialist"
                    }
                  />
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
            ) : null}
            {policy.proposals.map((proposal) => (
              <div
                key={proposal.id}
                className="flex items-center gap-2 text-12"
              >
                <span className="flex-1">
                  Proposed agent {proposal.agent_id}
                </span>
                {policy.can_manage ? (
                  <Button
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
                )}
              </div>
            ))}
          </section>
          <p className="text-12 text-muted-foreground">
            Only the person who requested work can decide its cards or stop it.
            The creator must transfer management to an accepting participant
            before leaving, or delete the group.
          </p>
          <div className="flex gap-2">
            <Button
              disabled={mutation.isPending}
              onClick={() =>
                setConfirm({
                  text: "Leave this group? You will lose access to its history and your queued work will be cancelled.",
                  change: { action: "leave" },
                })
              }
            >
              Leave group
            </Button>
            {policy.can_manage ? (
              <Button
                variant="destructive"
                disabled={mutation.isPending}
                onClick={() =>
                  setConfirm({
                    text: "Delete this group and its transcript for everyone? This cannot be undone.",
                    change: { action: "delete" },
                  })
                }
              >
                Delete group
              </Button>
            ) : null}
          </div>
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button onClick={onClose}>Close</Button>
        </DialogFooter>
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
      </DialogContent>
    </Dialog>
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
        onJoined(group);
      }
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="md:max-w-md">
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
            <>
              <p className="text-12">
                Created by {preview.creator_name ?? "a NyxID user"}
              </p>
              <p className="text-12">
                Participants:{" "}
                {preview.participants?.map((p) => p.display_name).join(", ")}
              </p>
              <p className="text-12">
                Expires {new Date(preview.expires_at).toLocaleString()}
              </p>
              <label className="flex items-center gap-2 text-12">
                <Checkbox
                  checked={disclosed}
                  onCheckedChange={(checked) => setDisclosed(checked === true)}
                />
                I understand everyone can read the full retained group history.
              </label>
            </>
          ) : null}
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            isLoading={busy}
            disabled={Boolean(preview && !disclosed)}
            onClick={() => void proceed()}
          >
            {preview ? "Accept invitation" : "Preview invitation"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

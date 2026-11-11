import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { ExecutionPermissionCard } from "./blocks/execution-permission-card";
import { Label } from "@/components/ui/label";
import { describeChanges } from "@/lib/grouped-form-changes";
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
  useUiTranslation();
  const orgs = useOrgs();
  const [org, setOrg] = useState("");
  const members = useOrgMembers(org);
  return (
    <GroupedList
      title={translateUi("Quick-add from your organization")}
      footer={translateUi("New participants can read the full retained group history.")}
    >
      <GroupedRow
        title={translateUi("Organization")}
        accessory={
          <Select value={org} onValueChange={setOrg}>
            <SelectTrigger aria-label={translateUi("Shared organization")}>
              <SelectValue placeholder={translateUi("Choose organization")} />
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
            title={person.display_name ?? translateUi("Member")}
            accessory={
              <Button
                variant="ghost"
                size="sm"
                disabled={busy || (group.participants?.length ?? 0) >= 16}
                onClick={() => onAdd(person.user_id)}
              >{translateUi("Add participant")}</Button>
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
  useUiTranslation();
  const form = useAppForm<z.infer<typeof renameSchema>>({
    mode: "onChange",
    resolver: zodResolver(renameSchema),
    defaultValues: { name },
  });
  const submit = form.handleSubmit(async (values) => {
    if (await save(values.name)) form.reset(values);
  });
  return (
    <Form {...form}>
      <form onSubmit={submit} className="space-y-6">
        <GroupedList title={translateUi("Profile")}>
          <FormField
            control={form.control}
            name="name"
            render={({ field }) => (
              <GroupedField>
                <GroupedLabel>{translateUi("Group name")}</GroupedLabel>
                <FormControl>
                  <Input {...field} maxLength={60} />
                </FormControl>
                <GroupedMessage />
              </GroupedField>
            )}
          />
        </GroupedList>
        <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "name", label: "Name" }])}
          disabled={!form.formState.isValid}
          label={translateUi("group name")}
          dirty={form.formState.isDirty}
          pending={busy}
          save={submit}
          discard={() => form.reset()}
          saveLabel={translateUi("Save")}
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
  useUiTranslation();
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
          <DialogTitle>{translateUi("Group participants and agents")}</DialogTitle>
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
              title={translateUi("Participants ({{value1}} of 16)", { value1: group.participants?.length ?? 0 })}
              aria-label={translateUi("Group participants")}
              footer={
                policy.transfer_to_user_id &&
                policy.transfer_to_user_id !== actor
                  ? translateUi("Waiting for the proposed creator to accept management.")
                  : undefined
              }
            >
              {group.participants?.map((person) => (
                <GroupedRow
                  key={person.id}
                  title={`${person.display_name}${person.id === actor ? " (you)" : ""}`}
                  accessory={
                    person.id === policy.creator_user_id ? (
                      <span className="text-13 text-text-tertiary">{translateUi("Creator")}</span>
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
                        >{translateUi("Transfer")}</Button>
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
                        >{translateUi("Remove")}</Button>
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
                  >{translateUi("Accept management")}</button>
                </GroupedRow>
              )}
            </GroupedList>
            {policy.can_manage && (
              <>
                <GroupedList
                  title={translateUi("Invitations")}
                  aria-label={translateUi("Group invitations")}
                  footer={
                    <>{translateUi("Single use, expires after seven days. A NyxID account is required. Send the link yourself.")}{invite && (
                        <p>{translateUi("Expires ")}{new Date(invite.expires_at).toLocaleString(getFormattingLocale())}
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
                    >{translateUi("Create invitation link")}</button>
                  </GroupedRow>
                  {invite && (
                    <>
                      <GroupedRow>
                        <Input
                          aria-label={translateUi("Invitation link")}
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
                        >{translateUi("Copy link")}</button>
                      </GroupedRow>
                    </>
                  )}
                  {policy.invites.map((item) => (
                    <GroupedRow
                      key={item.id}
                      title={translateUi("Invitation")}
                      description={translateUi("Expires {{value1}}", { value1: new Date(item.expires_at).toLocaleDateString(getFormattingLocale()) })}
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
                        >{translateUi("Cancel invitation")}</Button>
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
                      ? translateUi("Your requests use guest access and bill the agent owner.")
                      : agent.unavailable
                        ? groupAgentUnavailableReason(agent)
                        : translateUi("Your requests use your access and billing rules.")
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
                      title={translateUi("Participants can use this agent")}
                      accessory={
                        <Switch
                          aria-label={translateUi("Participants can use {{value1}}", { value1: agentTitle(agent) })}
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
                      >{translateUi("Disable participant access")}</button>
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
                      >{translateUi("Set as default recipient")}</button>
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
                      >{translateUi("Remove agent")}</button>
                    </GroupedRow>
                  )}
                </GroupedList>
              );
            })}
            {(eligible.length > 0 || policy.proposals.length > 0) && (
              <GroupedList title={translateUi("More agents")} aria-label={translateUi("Group agents")}>
                {eligible.length > 0 && (
                  <GroupedRow
                    title={
                      policy.can_manage
                        ? translateUi("Add a specialist")
                        : translateUi("Propose a specialist")
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
                          aria-label={translateUi("Add specialist")}
                          disabled={
                            mutation.isPending || group.members.length >= 8
                          }
                        >
                          <SelectValue placeholder={translateUi("Choose specialist")} />
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
                    title={translateUi("Proposed agent {{value1}}", { value1: proposal.agent_id })}
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
                        >{translateUi("Accept agent")}</Button>
                      ) : (
                        <span>{translateUi("Awaiting creator")}</span>
                      )
                    }
                  />
                ))}
              </GroupedList>
            )}
            <GroupedList
              title={translateUi("Group actions")}
              footer={
                <>
                  <p>{translateUi("Only the person who requested work can decide its cards or stop it. The creator must transfer management to an accepting participant before leaving, or delete the group.")}</p>
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
                >{translateUi("Leave group")}</button>
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
                  >{translateUi("Delete group")}</button>
                </GroupedRow>
              )}
            </GroupedList>
          </DialogBody>
          <GroupedFormBar dialog />
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
                <DialogTitle>{translateUi("Confirm group change")}</DialogTitle>
                <DialogDescription>{confirm.text}</DialogDescription>
              </DialogHeader>
              {error ? (
                <p role="alert" className="text-14 text-destructive">
                  {error}
                </p>
              ) : null}
              <DialogFooter>
                <Button onClick={() => setConfirm(undefined)}>{translateUi("Cancel")}</Button>
                <Button
                  variant="primary"
                  isLoading={mutation.isPending}
                  onClick={async () => {
                    if (await apply(confirm.change)) setConfirm(undefined);
                  }}
                >{translateUi("Confirm")}</Button>
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
  useUiTranslation();
  const mutation = useCollaborationDecision(group.id);
  return (
    <div className="mx-auto max-w-3xl space-y-2 px-4">
      {decisions.map((card, i) => card.execution_proposal && card.id && card.can_decide ? (
        <ExecutionPermissionCard key={card.id} requestId={card.id} proposal={card.execution_proposal} />
      ) : (
        <div
          key={card.id ?? `${card.requester_id}-${String(i)}`}
          className="space-y-2 rounded-lg border border-border bg-card p-3 text-14"
        >
          <p>
            {card.can_decide
              ? card.summary
              : translateUi("Waiting for {{value1}}", { value1: group.participants?.find((p) => p.id === card.requester_id)?.display_name ?? translateUi("participant") })}
          </p>
          {card.can_decide ? (
            <>
              <p className="text-muted-foreground">{card.audience}</p>
              {card.connection_id ? (
                <p>{translateUi("Your connection: ")}{card.connection_name ?? card.connection_id}
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
        <p role="alert" className="text-14 text-destructive">
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
  useUiTranslation();
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
            {preview ? translateUi("Join {{value1}}", { value1: preview.group_name }) : translateUi("Group invitation")}
          </DialogTitle>
          <DialogDescription>
            {preview?.history_disclosure ??
              translateUi("Preview this invitation before joining. A NyxID account is required.")}
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-3">
          {preview ? (
            <GroupedList
              title={translateUi("Invitation")}
              footer={translateUi("Expires {{value1}}", { value1: new Date(preview.expires_at).toLocaleString(getFormattingLocale()) })}
            >
              <GroupedRow
                title={translateUi("Created by")}
                accessory={preview.creator_name ?? "a NyxID user"}
              />
              <GroupedRow
                title={translateUi("Participants")}
                description={preview.participants
                  ?.map((p) => p.display_name)
                  .join(", ")}
              />
              <GroupedRow asChild interactive>
                <Label>
                  <Switch
                    className="order-last shrink-0"
                    checked={disclosed}
                    onCheckedChange={(checked) => setDisclosed(checked === true)}
                  />
                  <span className="min-w-0 flex-1">{translateUi("I understand everyone can read the full retained group history.")}</span>
                </Label>
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
          <GroupedDialogCancel disabled={busy}>{translateUi("Cancel")}</GroupedDialogCancel>
          <Button
            variant="primary"
            isLoading={busy}
            disabled={Boolean(preview && !disclosed)}
            onClick={() => void proceed()}
          >
            {preview ? translateUi("Accept invitation") : translateUi("Preview invitation")}
          </Button>
        </DialogFooter>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

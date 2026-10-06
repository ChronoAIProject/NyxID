import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useOrgs } from "@/hooks/use-orgs";
import { useOrgMembers } from "@/hooks/use-org-members";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import {
  useCreateNyxBotGroup,
  useDeleteNyxBotGroup,
  useUpdateNyxBotGroup,
} from "@/hooks/use-nyxbot-groups";
import { nyxBotOf } from "@/hooks/use-nyxbot-agents";
import { agentHandle, agentTitle } from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import {
  ASSISTANT_GROUP_MAX_MEMBERS,
  ASSISTANT_GROUP_NAME_MAX,
  assistantGroupFormSchema,
  type AssistantAgent,
  type AssistantAgentKind,
  type AssistantGroup,
  type AssistantGroupForm,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

export const GROUP_ROUTING_COPY =
  "Messages go to NyxBot (or the first agent) unless you @mention someone. " +
  "Agents hand work to each other the same way.";

interface PickerRow {
  readonly id: string;
  /** Display name, else handle. */
  readonly name: string;
  readonly display_name?: string | null;
  readonly kind: AssistantAgentKind;
  readonly description: string;
  readonly destroyed: boolean;
}

/**
 * Live agents to choose from, plus current members that were destroyed. The
 * server refuses destroyed agents in a member list, so those are shown
 * unchecked and leave the group on the next save.
 */
function pickerRows(
  agents: readonly AssistantAgent[],
  group?: AssistantGroup,
): PickerRow[] {
  const live = agents
    .filter((agent) => agent.status !== "destroyed")
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"))
    .map((agent) => ({
      id: agent.id,
      name: agentTitle(agent),
      display_name: agent.display_name,
      kind: agent.kind,
      description: [
        agentHandle(agent),
        agent.kind === "nyxbot" ? "Your personal agent" : agent.description || "Specialist",
      ]
        .filter(Boolean)
        .join(" · "),
      destroyed: false,
    }));
  // Until the agent list has loaded, only the group's own flag says who is gone.
  const known = agents.length > 0;
  const gone = (group?.members ?? [])
    .filter(
      (member) => member.destroyed || (known && !live.some((row) => row.id === member.id)),
    )
    .map((member) => ({
      id: member.id,
      name: member.name,
      kind: member.kind,
      description: `${member.destroyed ? "Destroyed" : "No longer available"} — removed when you save`,
      destroyed: true,
    }));
  return [...live, ...gone];
}

function AgentPicker({
  rows,
  value,
  onChange,
}: {
  readonly rows: readonly PickerRow[];
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  const full = value.length >= ASSISTANT_GROUP_MAX_MEMBERS;
  return (
    <ul
      aria-label="Agents"
      className="assistant-scrollbar max-h-56 space-y-0.5 overflow-y-auto rounded-lg border border-border p-1"
    >
      {rows.map((row) => {
        const id = `group-member-${row.id}`;
        const checked = value.includes(row.id);
        // Destroyed agents can never be (re)added.
        const disabled = row.destroyed || (!checked && full);
        return (
          <li key={row.id}>
            <label
              htmlFor={id}
              className={cn(
                "flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 text-12 hover:bg-overlay",
                row.destroyed && "opacity-50",
                disabled && "cursor-not-allowed",
              )}
            >
              <Checkbox
                id={id}
                checked={checked}
                disabled={disabled}
                onCheckedChange={(next) =>
                  onChange(
                    next === true
                      ? [...value, row.id]
                      : value.filter((member) => member !== row.id),
                  )
                }
              />
              <AgentAvatar agent={row} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-foreground">{row.name}</span>
                <span className="block truncate text-11 text-text-tertiary">
                  {row.description}
                </span>
              </span>
            </label>
          </li>
        );
      })}
    </ul>
  );
}

function GroupOwnerPicker({ value, onChange }: {
  readonly value?: string;
  readonly onChange: (id: string | undefined) => void;
}) {
  const orgs = useOrgs();
  const enabled = useFeature(FEATURE_FLAG.ORG_AGENTS);
  return <div className="space-y-2">
    <label htmlFor="group-owner" className="text-12 font-medium">Ownership</label>
    <Select value={value ?? "personal"} onValueChange={(id) => onChange(id === "personal" ? undefined : id)}>
      <SelectTrigger id="group-owner"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value="personal">Personal</SelectItem>
        {(orgs.data ?? []).filter((org) => ["admin", "member"].includes(org.your_role)).map((org) =>
          <SelectItem key={org.id} value={org.id} disabled={!enabled}>{org.display_name ?? org.slug}</SelectItem>)}
      </SelectContent>
    </Select>
    {!enabled && (orgs.data?.length ?? 0) > 0 ? <p className="text-11 text-muted-foreground">Organization groups are not enabled yet.</p> : null}
  </div>;
}

export function GroupParticipantPicker({ org, value, onChange }: {
  readonly org: string;
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  const members = useOrgMembers(org);
  const eligible = (members.data ?? []).filter((member) => !member.revoked_at && ["admin", "member"].includes(member.role));
  return <div className="space-y-2">
    <p className="text-12 font-medium">Participants ({value.length} of 16)</p>
    <ul aria-label="Participants" className="assistant-scrollbar max-h-44 space-y-1 overflow-y-auto rounded-lg border border-border p-2">
      {eligible.map((member) => <li key={member.user_id}>
        <label className="flex items-center gap-2 text-12">
          <Checkbox checked={value.includes(member.user_id)} disabled={value.length >= 16 && !value.includes(member.user_id)}
            onCheckedChange={(checked) => onChange(checked ? [...value, member.user_id] : value.filter((id) => id !== member.user_id))} />
          {member.display_name ?? "Member"}
        </label>
      </li>)}
    </ul>
    {members.isError ? <p role="alert" className="text-12 text-destructive">Could not load organization members.</p> : null}
    <p className="text-11 text-muted-foreground">Only selected Admins and Members can read and participate. The creator is included when creating a group.</p>
  </div>;
}

function GroupFields({
  form,
  rows,
}: {
  readonly form: ReturnType<typeof useAppForm<AssistantGroupForm>>;
  readonly rows: readonly PickerRow[];
}) {
  return (
    <>
      <FormField
        control={form.control}
        name="name"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Name</FormLabel>
            <FormControl>
              <Input
                placeholder="Launch crew"
                autoComplete="off"
                maxLength={ASSISTANT_GROUP_NAME_MAX}
                {...field}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormField
        control={form.control}
        name="member_agent_ids"
        render={({ field }) => (
          <FormItem>
            <FormLabel>
              Agents{" "}
              <span className="font-normal text-text-tertiary">
                ({field.value.length} of {ASSISTANT_GROUP_MAX_MEMBERS})
              </span>
            </FormLabel>
            <AgentPicker rows={rows} value={field.value} onChange={field.onChange} />
            <FormMessage />
          </FormItem>
        )}
      />
    </>
  );
}

/** Start a group chat: a name and its agents, NyxBot preselected. */
export function NewGroupDialog({
  agents,
  onClose,
  onCreated,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly onClose: () => void;
  readonly onCreated: (group: AssistantGroup) => void;
}) {
  const create = useCreateNyxBotGroup();
  const userId = useAuthStore((state) => state.user?.id);
  const nyxbotId = nyxBotOf(agents)?.id;
  const form = useAppForm<AssistantGroupForm>({
    resolver: zodResolver(assistantGroupFormSchema),
    defaultValues: { name: "", member_agent_ids: nyxbotId ? [nyxbotId] : [] },
  });
  const [error, setError] = useState<string>();
  const org = form.watch("org");
  const people = form.watch("participant_user_ids") ?? [];
  const name = form.watch("name");
  const members = form.watch("member_agent_ids");
  // Agents may still be loading when the dialog opens; preselect NyxBot once known.
  useEffect(() => {
    if (org || !nyxbotId || form.getValues("member_agent_ids").length) return;
    form.setValue("member_agent_ids", [nyxbotId], { shouldDirty: false, shouldTouch: false });
  }, [form, nyxbotId, org]);

  async function submit(values: AssistantGroupForm) {
    setError(undefined);
    try {
      onCreated(await create.mutateAsync(values));
    } catch (cause) {
      setError(errorMessage(cause, "Could not create the group. Try again."));
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !create.isPending) onClose();
      }}
    >
      <DialogContent scrollMode="body" className="md:max-w-md">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>New group</DialogTitle>
          <DialogDescription>
            Chat with up to {ASSISTANT_GROUP_MAX_MEMBERS} of your agents at once.{" "}
            {GROUP_ROUTING_COPY}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form noValidate onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-4 pb-1">
              <GroupOwnerPicker value={org} onChange={(id) => {
                form.setValue("org", id);
                form.setValue("member_agent_ids", id ? [] : nyxbotId ? [nyxbotId] : []);
                form.setValue("participant_user_ids", id && userId ? [userId] : undefined);
              }} />
              <GroupFields form={form} rows={pickerRows(agents.filter((agent) => org
                ? agent.owner_kind === "org" && agent.owner_id === org
                : true))} />
              {org ? <GroupParticipantPicker org={org} value={people} onChange={(ids) => form.setValue("participant_user_ids", ids)} /> : null}
              {error ? (
                <p role="alert" className="text-12 text-destructive">
                  {error}
                </p>
              ) : null}
            </DialogBody>
            <DialogFooter className="pt-4">
              <Button type="button" variant="ghost" onClick={onClose} disabled={create.isPending}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={create.isPending}
                disabled={!name.trim() || !members.length}
              >
                Create group
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

/** Rename a group, change its agents, or delete it. */
export function GroupSettingsDialog({
  group,
  agents,
  open,
  onOpenChange,
  onDeleted,
}: {
  readonly group: AssistantGroup;
  readonly agents: readonly AssistantAgent[];
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onDeleted: () => void;
}) {
  const update = useUpdateNyxBotGroup();
  const org = group.owner?.type === "org" ? group.owner.id : undefined;
  const canManage = !org || group.your_role === "creator" || group.your_role === "admin";
  const remove = useDeleteNyxBotGroup();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string>();
  const liveAgentIds = new Set(
    agents.filter((agent) => agent.status !== "destroyed").map((agent) => agent.id),
  );
  const alive = (member: AssistantGroup["members"][number]) =>
    !member.destroyed && (!agents.length || liveAgentIds.has(member.id));
  // Members that are gone leave on the next save (the server refuses them).
  const goneMembers = group.members.some((member) => !alive(member));
  const form = useAppForm<AssistantGroupForm>({
    resolver: zodResolver(assistantGroupFormSchema),
    defaultValues: {
      name: group.name,
      participant_user_ids: group.participants?.map((person) => person.id),
      lead_agent_id: org ? group.lead_agent_id : undefined,
      member_agent_ids: group.members.filter(alive).map((member) => member.id),
    },
  });
  const name = form.watch("name");
  const members = form.watch("member_agent_ids");
  const busy = update.isPending || remove.isPending;

  async function save(values: AssistantGroupForm) {
    setError(undefined);
    const dirty = form.formState.dirtyFields;
    try {
      await update.mutateAsync({
        id: group.id,
        ...(dirty.name ? { name: values.name } : {}),
        ...(org && dirty.participant_user_ids ? { participant_user_ids: values.participant_user_ids } : {}),
        ...(org && dirty.lead_agent_id ? { lead_agent_id: values.lead_agent_id } : {}),
        ...(dirty.member_agent_ids || goneMembers
          ? { member_agent_ids: values.member_agent_ids }
          : {}),
      });
      form.reset(values);
      onOpenChange(false);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save the group. Try again."));
    }
  }

  async function leaveGroup() {
    try {
      await update.mutateAsync({ id: group.id, leave: true });
      onDeleted();
    } catch (cause) { setError(errorMessage(cause, "Could not leave the group.")); }
  }

  async function deleteGroup() {
    setError(undefined);
    try {
      await remove.mutateAsync(group.id);
      setConfirmDelete(false);
      onDeleted();
    } catch (cause) {
      setError(errorMessage(cause, "Could not delete the group. Try again."));
    }
  }

  return (
    <>
      <Dialog
        open={open && !confirmDelete}
        onOpenChange={(next) => {
          if (!busy) onOpenChange(next);
        }}
      >
        <DialogContent scrollMode="body" className="md:max-w-md">
          <DialogHeader className="shrink-0 pr-6">
            <DialogTitle>Group settings</DialogTitle>
            <DialogDescription>{GROUP_ROUTING_COPY}</DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form aria-label="Group settings" noValidate onSubmit={form.handleSubmit(save)}>
              <DialogBody className="space-y-4 pb-1">
                {org ? <p className="text-12 text-muted-foreground">{group.owner?.name} · {canManage ? "You can manage this group." : "Only the creator and participating Admins manage this group."}</p> : null}
                <fieldset disabled={!canManage} className="space-y-4">
                  <GroupFields form={form} rows={pickerRows(agents.filter((agent) => org ? agent.owner_id === org : true), group)} />
                  {org ? <>
                    <GroupParticipantPicker org={org} value={form.watch("participant_user_ids") ?? []} onChange={(ids) => form.setValue("participant_user_ids", ids)} />
                    <div className="space-y-2">
                      <label htmlFor="group-lead" className="text-12 font-medium">Lead agent</label>
                      <Select value={form.watch("lead_agent_id")} onValueChange={(id) => form.setValue("lead_agent_id", id)}>
                        <SelectTrigger id="group-lead"><SelectValue /></SelectTrigger>
                        <SelectContent>{agents.filter((agent) => members.includes(agent.id)).map((agent) => <SelectItem key={agent.id} value={agent.id}>{agentTitle(agent)}</SelectItem>)}</SelectContent>
                      </Select>
                    </div>
                  </> : null}
                </fieldset>
                {error ? (
                  <p role="alert" className="text-12 text-destructive">
                    {error}
                  </p>
                ) : null}
              </DialogBody>
              <DialogFooter className="pt-4">
                <Button
                  type="button"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => onOpenChange(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  isLoading={update.isPending}
                  disabled={
                    !canManage || !(form.formState.isDirty || goneMembers) || !name.trim() || !members.length
                  }
                >
                  Save
                </Button>
              </DialogFooter>
            </form>
          </Form>
          {org ? <div className="space-y-2">
            <p className="text-12 text-muted-foreground">
              The creator or a participating Admin must remain. Leaving as the last participant deletes the group.
              Wait for your running turns to finish before leaving or being removed.
            </p>
            <Button type="button" variant="outline" disabled={busy} onClick={() => void leaveGroup()}>Leave group</Button>
          </div> : null}
          {canManage ? <section
            aria-label="Danger zone"
            className="mt-2 flex items-center justify-between gap-4 rounded-xl border border-destructive/40 p-4"
          >
            <div className="space-y-1">
              <h3 className="text-13 font-semibold text-destructive">Delete group</h3>
              <p className="text-12 text-destructive/70">
                Removes the group and its messages. Your agents and their own threads stay.
              </p>
            </div>
            <Button
              type="button"
              size="sm"
              variant="destructive"
              disabled={busy}
              onClick={() => {
                setError(undefined);
                setConfirmDelete(true);
              }}
            >
              Delete
            </Button>
          </section> : null}
        </DialogContent>
      </Dialog>
      <Dialog
        open={open && confirmDelete}
        onOpenChange={(next) => {
          if (!next && !remove.isPending) setConfirmDelete(false);
        }}
      >
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete {group.name}?</DialogTitle>
            <DialogDescription>
              The group and its messages are deleted. This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              variant="ghost"
              disabled={remove.isPending}
              onClick={() => setConfirmDelete(false)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              isLoading={remove.isPending}
              onClick={() => void deleteGroup()}
            >
              Delete group
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

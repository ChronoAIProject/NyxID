import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label, FieldHelp } from "@/components/ui/label";
import { describeChanges, changeList } from "@/lib/grouped-form-changes";
import { flushSync } from "react-dom";
import { useCollaborationAvailable } from "@/hooks/use-collaboration-groups";
import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
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
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
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
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import { ChangeReview } from "@/components/ui/change-review";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
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
  "Messages go to the recipients shown above the composer: the agents you " +
  "@mention or pick, else whoever you last addressed, else NyxBot (or the first " +
  "agent). Agents hand work to each other by @mentioning them.";

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
    .filter((agent) => agent.status !== "destroyed" && agent.can_use !== false
      && !group?.members.some((member) => member.id === agent.id && member.unavailable))
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"))
    .map((agent) => ({
      id: agent.id,
      name: agentTitle(agent),
      display_name: agent.display_name,
      kind: agent.kind,
      description: [
        agentHandle(agent),
        agent.share_role ? `Shared agent${agent.owner_name ? ` · ${agent.owner_name}` : ""}` : agent.kind === "nyxbot" ? "Your personal agent" : agent.description || "Specialist",
      ]
        .filter(Boolean)
        .join(" · "),
      destroyed: false,
    }));
  // Until the agent list has loaded, only the group's own flag says who is gone.
  const known = agents.length > 0;
  const gone = (group?.members ?? [])
    .filter(
      (member) => member.destroyed || member.unavailable || (known && !live.some((row) => row.id === member.id)),
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
  footer,
}: {
  readonly footer?: React.ReactNode;
  readonly rows: readonly PickerRow[];
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  useUiTranslation();
  const full = value.length >= ASSISTANT_GROUP_MAX_MEMBERS;
  const [search, setSearch] = useState("");
  return (
    <GroupedList
      title={translateUi("Agents ({{value1}} of {{value2}})", { value1: value.length, value2: ASSISTANT_GROUP_MAX_MEMBERS })}
      footer={footer}
    >
      {rows.length > 8 && (
        <GroupedRow>
          <Input
            aria-label={translateUi("Search agents")}
            placeholder={translateUi("Search agents")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </GroupedRow>
      )}
      <ul
        aria-label={translateUi("Agents")}
        className="assistant-scrollbar max-h-72 overflow-y-auto"
      >
        {rows
          .filter((row) =>
            `${row.name} ${row.description}`
              .toLowerCase()
              .includes(search.toLowerCase()),
          )
          .map((row) => {
            const id = `group-member-${row.id}`;
            const checked = value.includes(row.id);
            // Destroyed agents can never be (re)added.
            const disabled = row.destroyed || (!checked && full);
            return (
              <li key={row.id}>
                <GroupedRow asChild interactive>
                  <Label
                    htmlFor={id}
                    className={cn(
                      disabled && "cursor-not-allowed text-text-tertiary",
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
                      <FieldHelp className="block truncate text-text-tertiary">
                        {row.description}
                      </FieldHelp>
                    </span>
                  </Label>
                </GroupedRow>
              </li>
            );
          })}
      </ul>
    </GroupedList>
  );
}

function GroupOwnerPicker({
  value,
  onChange,
}: {
  readonly value?: string;
  readonly onChange: (id: string | undefined) => void;
}) {
  useUiTranslation();
  const orgs = useOrgs();
  const enabled = useFeature(FEATURE_FLAG.ORG_AGENTS);
  return (
    <GroupedList
      footer={
        !enabled && (orgs.data?.length ?? 0) > 0
          ? translateUi("Organization groups are not enabled yet.")
          : undefined
      }
    >
      <GroupedRow>
        <Label htmlFor="group-owner" className="flex-1">{translateUi("Ownership")}</Label>
        <Select value={value ?? "personal"} onValueChange={(id) => onChange(id === "personal" ? undefined : id)}>
          <SelectTrigger id="group-owner"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="personal">{translateUi("Personal")}</SelectItem>
            {(orgs.data ?? []).filter((org) => ["admin", "member"].includes(org.your_role)).map((org) =>
              <SelectItem key={org.id} value={org.id} disabled={!enabled}>{org.display_name ?? org.slug}</SelectItem>)}
          </SelectContent>
        </Select>
      </GroupedRow>
    </GroupedList>
  );
}

export function GroupParticipantPicker({
  org,
  value,
  onChange,
}: {
  readonly org: string;
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  useUiTranslation();
  const members = useOrgMembers(org);
  const eligible = (members.data ?? []).filter((member) => !member.revoked_at && ["admin", "member"].includes(member.role));
  const [search, setSearch] = useState("");
  return (
    <GroupedList
      title={translateUi("Participants ({{value1}} of 16)", { value1: value.length })}
      footer={
        <>
          {members.isError && (
            <p role="alert" className="text-destructive">{translateUi("Could not load organization members.")}</p>
          )}
          <p>{translateUi("Only selected Admins and Members can read and participate. The creator is included when creating a group.")}</p>
        </>
      }
    >
      {eligible.length > 8 && (
        <GroupedRow>
          <Input
            aria-label={translateUi("Search participants")}
            placeholder={translateUi("Search participants")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </GroupedRow>
      )}
      <ul
        aria-label={translateUi("Participants")}
        className="assistant-scrollbar max-h-64 overflow-y-auto"
      >
        {eligible
          .filter((member) =>
            (member.display_name ?? "Member")
              .toLowerCase()
              .includes(search.toLowerCase()),
          )
          .map((member) => (
            <li key={member.user_id}>
              <GroupedRow asChild interactive>
                <Label>
                  <Checkbox checked={value.includes(member.user_id)} disabled={value.length >= 16 && !value.includes(member.user_id)}
                    onCheckedChange={(checked) => onChange(checked ? [...value, member.user_id] : value.filter((id) => id !== member.user_id))} />
                  {member.display_name ?? translateUi("Member")}
                </Label>
              </GroupedRow>
            </li>
          ))}
      </ul>
    </GroupedList>
  );
}

function GroupFields({
  form,
  rows,
  memberFooter,
}: {
  readonly form: ReturnType<typeof useAppForm<AssistantGroupForm>>;
  readonly rows: readonly PickerRow[];
  readonly memberFooter?: React.ReactNode;
}) {
  useUiTranslation();
  return (
    <>
      <FormField
        control={form.control}
        name="name"
        render={({ field }) => (
          <GroupedList title={translateUi("Profile")}>
            <GroupedField>
              <GroupedLabel>{translateUi("Name")}</GroupedLabel>
              <FormControl>
                <Input
                  placeholder={translateUi("Launch crew")}
                  autoComplete="off"
                  maxLength={ASSISTANT_GROUP_NAME_MAX}
                  {...field}
                />
              </FormControl>
              <GroupedMessage />
            </GroupedField>
          </GroupedList>
        )}
      />
      <FormField
        control={form.control}
        name="member_agent_ids"
        render={({ field }) => (
          <FormItem>
            <AgentPicker
              rows={rows}
              value={field.value}
              onChange={field.onChange}
              footer={
                <>
                  {memberFooter}
                  <FormMessage className="text-13" />
                </>
              }
            />
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
  useUiTranslation();
  const create = useCreateNyxBotGroup();
  const collaborationAvailable = useCollaborationAvailable();
  const userId = useAuthStore((state) => state.user?.id);
  const nyxbotId = nyxBotOf(agents)?.id;
  const form = useAppForm<AssistantGroupForm>({
    mode: "onChange",
    resolver: zodResolver(assistantGroupFormSchema),
    defaultValues: { name: "", member_agent_ids: nyxbotId ? [nyxbotId] : [] },
  });
  const [error, setError] = useState<string>();
  const org = form.watch("org");
  const collaboration = form.watch("collaboration");
  const people = form.watch("participant_user_ids") ?? [];
  // Agents may still be loading when the dialog opens; preselect NyxBot once known.
  useEffect(() => {
    if (org || collaboration || !nyxbotId || form.getValues("member_agent_ids").length) return;
    form.setValue("member_agent_ids", [nyxbotId], { shouldDirty: false, shouldTouch: false });
  }, [form, nyxbotId, org, collaboration]);

  async function submit(values: AssistantGroupForm) {
    setError(undefined);
    try {
      const created = await create.mutateAsync(values);
      flushSync(() => form.reset(values));
      onCreated(created);
    } catch (cause) {
      setError(errorMessage(cause, translateUi("Could not create the group. Try again.")));
    }
  }

  return (
    <GroupedDialog
      open
      onOpenChange={(open) => {
        if (!open && !create.isPending) onClose();
      }}
    >
      <GroupedDialogContent scrollMode="body" className="md:max-w-md">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>{translateUi("New crew")}</DialogTitle>
          <DialogDescription>{translateUi("Chat with up to ")}{ASSISTANT_GROUP_MAX_MEMBERS}{translateUi(" of your agents at once. ")}{translateUi(GROUP_ROUTING_COPY)}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form noValidate onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-6 pb-4">
              <GroupOwnerPicker value={org} onChange={(id) => {
                form.setValue("org", id);
                form.setValue("collaboration", undefined);
                form.setValue("member_agent_ids", id ? [] : nyxbotId ? [nyxbotId] : []);
                form.setValue("participant_user_ids", id && userId ? [userId] : undefined);
              }} />
              {!org && collaborationAvailable.data ? (
                <GroupedList
                  title={translateUi("People")}
                  footer={
                    collaboration
                      ? translateUi("Up to 16 people can join by invitation after creation. Everyone can read the full retained group history. Personal NyxBot cannot join.")
                      : undefined
                  }
                >
                  <GroupedRow asChild interactive>
                    <Label>
                      <Switch className="order-last shrink-0" checked={Boolean(collaboration)} onCheckedChange={(checked) => {
                        form.setValue("collaboration", checked === true);
                        form.setValue("member_agent_ids", []);
                      }} />
                      <span className="min-w-0 flex-1">{translateUi("Invite other people")}</span>
                    </Label>
                  </GroupedRow>
                </GroupedList>
              ) : null}
              <GroupFields form={form} rows={pickerRows(agents.filter((agent) => org
                ? agent.owner_kind === "org" && agent.owner_id === org
                : !collaboration || agent.kind === "specialist"))} />
              {org ? (
                <GroupParticipantPicker org={org} value={people} onChange={(ids) => form.setValue("participant_user_ids", ids)} />
              ) : null}
              {error ? (
                <GroupedList
                  footer={
                    <p role="alert" className="text-destructive">
                      {translateUi(error)}
                    </p>
                  }
                >
                  <GroupedRow title={translateUi("Crew could not be created")} />
                </GroupedList>
              ) : null}
            </DialogBody>
            <GroupedDialogActions
              intent="create"
              label={translateUi("new crew")}
              dirty={form.formState.isDirty}
              pending={create.isPending}
              disabled={!form.formState.isValid}
              save={form.handleSubmit(submit)}
              discard={() => {
                form.reset();
                setError(undefined);
              }}
              saveLabel={translateUi("Create crew")}
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
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
  useUiTranslation();
  const update = useUpdateNyxBotGroup();
  const org = group.owner?.type === "org" ? group.owner.id : undefined;
  const canManage = !org || group.your_role === "creator" || group.your_role === "admin";
  const remove = useDeleteNyxBotGroup();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string>();
  const liveAgentIds = new Set(
    agents.filter((agent) => agent.status !== "destroyed" && agent.can_use !== false).map((agent) => agent.id),
  );
  const alive = (member: AssistantGroup["members"][number]) =>
    !member.destroyed && !member.unavailable && (!agents.length || liveAgentIds.has(member.id));
  // Members that are gone leave on the next save (the server refuses them).
  const goneMembers = group.members.some((member) => !alive(member));
  const form = useAppForm<AssistantGroupForm>({
    mode: "onChange",
    resolver: zodResolver(assistantGroupFormSchema),
    defaultValues: {
      name: group.name,
      participant_user_ids: group.participants?.map((person) => person.id),
      lead_agent_id: org ? group.lead_agent_id : undefined,
      member_agent_ids: group.members.filter(alive).map((member) => member.id),
    },
  });
  const members = form.watch("member_agent_ids");
  const busy = update.isPending || remove.isPending;
  const changes = () => describeChanges({ ...form.formState.defaultValues, member_agent_ids: group.members.map((member) => member.id) }, form.getValues(), [
    { key: "name", label: "Name" },
    { key: "member_agent_ids", label: "Agents", list: (value) => changeList(value).map((id) => { const agent = agents.find((agent) => agent.id === id); return agent ? agentTitle(agent) : group.members.find((member) => member.id === id)?.name || "Unavailable agent"; }) },
    { key: "participant_user_ids", label: "People", list: (value) => changeList(value).map((id) => { const person = group.participants?.find((person) => person.id === id); return person?.display_name || person?.handle || id; }) },
    { key: "lead_agent_id", label: "Lead agent", format: (value) => agents.find((agent) => agent.id === value)?.name || String(value || "None") },
  ]);

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
      return true;
    } catch (cause) {
      setError(errorMessage(cause, translateUi("Could not save the group. Try again.")));
      return false;
    }
  }

  async function leaveGroup() {
    try {
      await update.mutateAsync({ id: group.id, leave: true });
      onDeleted();
    } catch (cause) { setError(errorMessage(cause, translateUi("Could not leave the group."))); }
  }

  async function deleteGroup() {
    setError(undefined);
    try {
      await remove.mutateAsync(group.id);
      setConfirmDelete(false);
      onDeleted();
    } catch (cause) {
      setError(errorMessage(cause, translateUi("Could not delete the group. Try again.")));
    }
  }

  return (
    <>
      <GroupedDialog
        open={open}
        onOpenChange={(next) => {
          if (!busy) onOpenChange(next);
        }}
      >
        <GroupedDialogContent scrollMode="body" className="md:max-w-md">
          <DialogHeader className="shrink-0 pr-6">
            <DialogTitle>{translateUi("Group settings")}</DialogTitle>
            <DialogDescription>{translateUi(GROUP_ROUTING_COPY)}</DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form
              aria-label={translateUi("Group settings")}
              noValidate
              onSubmit={form.handleSubmit(save)}
            >
              <DialogBody className="space-y-6 pb-4">
                {org ? (
                  <GroupedList
                    title={translateUi("Ownership")}
                    footer={
                      canManage
                        ? translateUi("You can manage this group.")
                        : translateUi("Only the creator and participating Admins manage this group.")
                    }
                  >
                    <GroupedRow title={group.owner?.name} />
                  </GroupedList>
                ) : null}
                <fieldset disabled={!canManage} className="min-w-0 space-y-6">
                  <GroupFields
                    form={form}
                    rows={pickerRows(agents.filter((agent) => org ? agent.owner_id === org : true), group)}
                    memberFooter={!org ? (
                      <p>{translateUi("Removing an agent stops its crew work and deletes its private crew thread. Re-adding starts a new thread. Published crew messages stay.")}</p>
                    ) : undefined}
                  />
                  {org ? (
                    <>
                      <GroupParticipantPicker org={org} value={form.watch("participant_user_ids") ?? []} onChange={(ids) => form.setValue("participant_user_ids", ids)} />
                      <GroupedList title={translateUi("Routing")}>
                        <GroupedRow>
                          <Label
                            htmlFor="group-lead"
                            className="flex-1"
                          >{translateUi("Lead agent")}</Label>
                          <Select value={form.watch("lead_agent_id")} onValueChange={(id) => form.setValue("lead_agent_id", id)}>
                            <SelectTrigger id="group-lead"><SelectValue /></SelectTrigger>
                            <SelectContent>{agents.filter((agent) => members.includes(agent.id)).map((agent) => <SelectItem key={agent.id} value={agent.id}>{agentTitle(agent)}</SelectItem>)}</SelectContent>
                          </Select>
                        </GroupedRow>
                      </GroupedList>
                    </>
                  ) : null}
                </fieldset>
                {goneMembers && !form.formState.isDirty && canManage && (
                  <GroupedList
                    title={translateUi("Unavailable agents")}
                    footer={translateUi("Unavailable agents will be removed on the next save.")}
                  >
                    <ChangeReview entries={[{ label: "group settings", dirty: true, changes, save: form.handleSubmit(save), discard: () => {} }]} disabled={busy || !members.length} save={async () => {
                      let saved = false;
                      await form.handleSubmit(async (values) => { saved = await save(values); })();
                      return saved ? ["group settings"] : [];
                    }}>{(review) => <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={busy || !members.length}
                        onClick={review}
                      >{translateUi("Remove unavailable agents")}</button>
                    </GroupedRow>}</ChangeReview>
                  </GroupedList>
                )}
                <GroupedList
                  title={translateUi("Group actions")}
                  footer={
                    <>
                      {org && (
                        <p>{translateUi("The creator or a participating Admin must remain. Leaving as the last participant deletes the group. Wait for your running turns to finish before leaving or being removed.")}</p>
                      )}
                      {canManage && (
                        <p>{translateUi("Delete removes the group and its messages. Your agents and their own threads stay.")}</p>
                      )}
                    </>
                  }
                >
                  {org && (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        className="text-destructive"
                        disabled={busy}
                        onClick={() => void leaveGroup()}
                      >{translateUi("Leave group")}</button>
                    </GroupedRow>
                  )}
                  {canManage && (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        className="text-destructive"
                        disabled={busy}
                        onClick={() => {
                          setError(undefined);
                          setConfirmDelete(true);
                        }}
                      >{translateUi("Delete")}</button>
                    </GroupedRow>
                  )}
                </GroupedList>
                {error ? (
                  <GroupedList
                    footer={
                      <p role="alert" className="text-destructive">
                        {translateUi(error)}
                      </p>
                    }
                  >
                    <GroupedRow title={translateUi("Group could not be saved")} />
                  </GroupedList>
                ) : null}
              </DialogBody>
              <GroupedDialogActions
                changes={changes}
                label={translateUi("group settings")}
                dirty={form.formState.isDirty}
                pending={busy}
                disabled={!canManage || !form.formState.isValid}
                save={form.handleSubmit(save)}
                discard={() => {
                  form.reset();
                  setError(undefined);
                }}
                saveLabel={translateUi("Save")}
              />
            </form>
          </Form>
        </GroupedDialogContent>
      </GroupedDialog>
      <Dialog
        open={open && confirmDelete}
        onOpenChange={(next) => {
          if (!next && !remove.isPending) setConfirmDelete(false);
        }}
      >
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>{translateUi("Delete ")}{group.name}?</DialogTitle>
            <DialogDescription>{translateUi("The group and its messages are deleted. This cannot be undone.")}</DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-14 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              variant="ghost"
              disabled={remove.isPending}
              onClick={() => setConfirmDelete(false)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={remove.isPending}
              onClick={() => void deleteGroup()}
            >{translateUi("Delete group")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

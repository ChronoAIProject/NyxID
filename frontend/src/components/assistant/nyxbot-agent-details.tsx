import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { AgentSkills } from "./agent-skills";
import { AgentLearningReview } from "./agent-learning-review";
import { MachineCapabilities } from "./machine-capabilities";
import { AgentOperationScopes } from "./agent-operation-scopes";
import { MachineGrantPicker } from "./machine-grant-picker";
import { AgentAutomations } from "./automation-preferences";
import { useEffect, useState, type ReactNode } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AgentKindBadge } from "@/components/assistant/nyxbot-agent-panels";
import { ChannelBotsManager } from "@/components/assistant/nyxbot-channels";
import {
  PERSONA_HINT,
  ServiceGrantPicker,
  TEXTAREA_CLASS,
} from "@/components/assistant/nyxbot-agent-forms";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import {
  nyxBotQueryKeys,
  useDeleteNyxBotAgent,
  useDestroyNyxBotAgent,
  useForgetNyxBotMemory,
  useRememberNyxBotMemory,
  useNyxBotAgent,
  useNyxBotSettings,
  useSetNyxBotAgentGrants,
  useUpdateNyxBotAgent,
} from "@/hooks/use-nyxbot-agents";
import { ApiError } from "@/lib/api-client";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type { AgentDetailsTab } from "@/lib/assistant/search";
import {
  ACCESS_MODE_DESCRIPTION,
  ACCESS_MODE_LABEL,
  AGENT_STATUS_LABEL,
  agentHandle,
  agentTitle,
  effectiveAccessMode,
} from "@/lib/assistant/nyxbot-labels";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { ModelSelect } from "@/components/assistant/nyxbot-access-controls";
import { formatDateTime } from "@/lib/utils";
import {
  ASSISTANT_AGENT_DISPLAY_NAME_MAX,
  ASSISTANT_MEMORY_NOTE_MAX,
  assistantMemoryNoteSchema,
  ASSISTANT_AGENT_PERSONA_MAX,
  assistantAgentGrantsSchema,
  assistantAgentProfileSchema,
  nyxAgentAccessModeSchema,
  type AssistantAgent,
  type AssistantAgentGrants,
  type AssistantAgentMemoryNote,
  type AssistantGuestAccess,
  type AssistantAgentProfile,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function Section({
  title,
  description,
  children,
  id,
}: {
  readonly title: string;
  readonly description?: string;
  readonly children?: ReactNode;
  readonly id?: string;
}) {
  return (
    <section
      id={id}
      tabIndex={id ? -1 : undefined}
      aria-label={title}
      className="space-y-3 outline-none"
    >
      <div className="space-y-1">
        <h3 className="text-13 font-semibold text-foreground">{title}</h3>
        {description ? (
          <p className="text-12 text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {children}
    </section>
  );
}

/**
 * Who can widen a specialist's access, and why the agent itself cannot. Org
 * agents are maintained by the organization: only its maintainers approve.
 */
export function accessAuthorityCopy(
  agent: AssistantAgent,
  requestMode: "denied" | "routed_to_nyxbot" | undefined,
): string {
  const sandbox = translateUi("This specialist is sandboxed: it cannot grant access to itself or to other agents.");
  const direct = translateUi("When you talk to it directly, it tells you what it is missing and continues; you grant access below.");
  if (agent.owner_kind === "org") {
    const org = agent.owner_name ?? translateUi("the organization");
    return agent.can_maintain === false
      ? translateUi("{{sandbox}} Only a maintainer of {{org}} (an Admin or Member) can approve or grant its access; you can view it but not change it.", { sandbox, org })
      : translateUi("{{sandbox}} Maintainers of {{org}}, including you, approve and grant its access here; your personal NyxBot cannot approve it on the organization's behalf.", { sandbox, org });
  }
  return requestMode === "routed_to_nyxbot"
    ? translateUi("{{sandbox}} {{direct}} Only for work NyxBot assigned, or in a group with NyxBot, do its requests go to NyxBot, which approves or denies them.", { sandbox, direct })
    : translateUi("{{sandbox}} {{direct}} Its requests are never routed: only work NyxBot assigned, or work in a group NyxBot is in, reports the missing access to NyxBot instead.", { sandbox, direct });
}

const GRANTS_SECTION_ID = "agent-details-grants";
const SKILLS_SECTION_ID = "agent-details-skills";

function isNoAccess(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 403 || error.status === 404);
}

/** Says why the forms are read-only instead of leaving a silently dead Save button. */
function ReadOnlyNote({ agent }: { readonly agent: AssistantAgent }) {
  const { t } = useUiTranslation();
  if (agent.status === "destroyed") {
    return (
      <section
        role="note"
        aria-label={t("Agent destroyed")}
        className="space-y-1 rounded-xl border border-dashed border-border px-4 py-3 text-12 text-muted-foreground"
      >
        <p className="font-medium text-foreground">
          {agentTitle(agent)}{t(" was destroyed")}{agent.destroyed_at ? t(" on {{value}}", { value: formatDateTime(agent.destroyed_at) }) : ""}.
        </p>
        <p>{t("Its grants, skills and settings are read-only and it no longer runs.")}{agent.kind === "specialist" && agent.can_maintain !== false
            ? t(" You can delete it permanently below.")
            : ""}
        </p>
      </section>
    );
  }
  if (agent.can_maintain === false) {
    const org = agent.owner_name ?? translateUi("the organization");
    return (
      <section
        role="note"
        aria-label={t("Read-only")}
        className="space-y-1 rounded-xl border border-dashed border-border px-4 py-3 text-12 text-muted-foreground"
      >
        <p className="font-medium text-foreground">{t("Read-only.")}</p>
        <p>{t("Only a maintainer of ")}{org}{t(" (an Admin or Member) can change its grants, skills and settings. You can view them.")}</p>
      </section>
    );
  }
  return null;
}

/**
 * Everything about one agent: its role, grants, memory, channel bots and
 * lifecycle. Opened from the thread header.
 */
export function AgentDetailsSheet({
  agentId,
  agents,
  proposalId,
  tab,
  open,
  onOpenChange,
  onDeleted,
}: {
  readonly agentId: string | undefined;
  readonly agents: readonly AssistantAgent[];
  readonly proposalId?: string;
  readonly tab?: AgentDetailsTab;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onDeleted: () => void;
}) {
  const { t } = useUiTranslation();
  const detail = useNyxBotAgent(agentId, open);
  const agent = detail.data?.agent;
  const loadedAgentId = agent?.id;
  const requestMode = useNyxBotSettings(open).data?.specialist_access_requests;
  const noAccess = detail.error ? isNoAccess(detail.error) : false;
  useEffect(() => {
    if (!open || !tab || !loadedAgentId) return;
    const target = document.getElementById(tab === "grants" ? GRANTS_SECTION_ID : SKILLS_SECTION_ID);
    if (!target) return;
    // jsdom has no scrollIntoView.
    if (typeof target.scrollIntoView === "function") target.scrollIntoView({ block: "start" });
    target.focus({ preventScroll: true });
  }, [open, tab, loadedAgentId]);
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader className="space-y-1.5 pr-6">
          <SheetTitle>{t("Agent details")}</SheetTitle>
          <SheetDescription>
            {agent
              ? agent.kind === "nyxbot"
                ? t("Your personal agent. It has full access and remembers across all its threads and chat apps.")
                : t("A specialist with its own memory. It uses only the services, machines and skills granted here and handles the work you give it directly itself.")
              : noAccess
                ? t("This agent is not available to you.")
                : t("Loading agent...")}
          </SheetDescription>
        </SheetHeader>
        <div className="mt-5 space-y-8">
          {detail.error ? (
            noAccess ? (
              <section
                role="note"
                aria-label={t("No access")}
                className="space-y-1 rounded-xl border border-dashed border-border px-4 py-3 text-12 text-muted-foreground"
              >
                <p className="font-medium text-foreground">{t("You cannot view this agent.")}</p>
                <p>{t("It belongs to an organization you are not a member of, or it no longer exists. Its grants and skills cannot be shown or changed here.")}</p>
              </section>
            ) : (
              <ErrorBanner
                message={t("Could not load this agent. {{message}}", { message: translateErrorMessage(detail.error.message) })}
                onRetry={() => void detail.refetch()}
              />
            )
          ) : null}
          {agent ? (
            <>
              <div className="flex flex-wrap items-center gap-2 text-11 text-text-tertiary">
                <AgentAvatar agent={agent} size="lg" />
                <span className="text-15 font-semibold text-foreground">
                  {agentTitle(agent)}
                </span>
                {agentHandle(agent) ? <span>{agentHandle(agent)}</span> : null}
                <AgentKindBadge kind={agent.kind} />
                <Badge variant={agent.status === "running" ? "success" : "secondary"}>
                  {AGENT_STATUS_LABEL[agent.status]}
                </Badge>
                <span>
                  {agent.kind === "specialist"
                    ? t("Created by {{value}} · ", { value: agent.created_by === "nyxbot" ? t("NyxBot") : t("you") })
                    : ""}
                  {agent.memory_count}{" "}
                  {agent.memory_count === 1 ? t("memory") : t("memories")}
                </span>
              </div>
              <ReadOnlyNote agent={agent} />
              <Section
                title={t("Ownership")}
                description={
                  agent.owner_kind === "org"
                    ? t("{{value}} · Maintained by Admins and Members. Your role: {{value1}}.{{value2}}", { value: agent.owner_name ?? agent.owner_id, value1: agent.org_role ?? t("member"), value2: agent.can_maintain === false ? t(" Read-only; viewers cannot maintain or chat.") : t(" Chats are private to you; memory is shared with the organization.") })
                    : t("Personal agent")
                }
              />
              <ProfileForm
                key={[agent.id, agent.name, agent.description, agent.display_name, agent.persona].join(
                  ":",
                )}
                agent={agent}
              />
              {agent.kind === "specialist" ? (
                <>
                  <Section title={t("Access authority")} description={accessAuthorityCopy(agent, requestMode)} />
                  <GrantsForm
                    key={`${agent.id}:${agent.services.join(",")}:${String(agent.account_read)}:${JSON.stringify(agent.guest_access)}`}
                    agent={agent}
                  />
                </>
              ) : (
                <Section id={GRANTS_SECTION_ID} title={t("Access")} description={t("NyxBot runs with full access to your connected services and account, and is the only agent that can grant access to specialists. Machine capabilities are assigned separately. Its access mode decides when it asks you first.")}>
                  <MachineCapabilities agentId={agent.id} disabled={agent.status === "destroyed"} />
                </Section>
              )}
              <AgentSkills id={SKILLS_SECTION_ID} agentId={agent.id} readOnly={agent.status === "destroyed" || agent.can_maintain === false
                } />
              <AgentLearningReview key={`${agent.id}:${proposalId ?? ""}`} agentId={agent.id} proposalId={proposalId} readOnly={agent.status === "destroyed" || agent.can_maintain === false} />
              {agent.can_use !== false ? (
                <AgentAutomations agentId={agent.id} />
              ) : null}
              {agent.can_maintain !== false ? (
                <MemoryList agent={agent} memory={detail.data?.memory ?? []} />
              ) : null}
              {agent.status === "destroyed" ||
              agent.owner_kind === "org" ? null : (
                <Section
                  title={t("Channel bots")}
                  description={t("Chat with {{value}} from these bots.", { value: agentTitle(agent) })}
                >
                  <ChannelBotsManager agents={agents} agent={agent} />
                </Section>
              )}
              {agent.kind === "specialist" && agent.can_maintain !== false ? (
                <Lifecycle agent={agent} onDeleted={onDeleted} />
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}

/** The access-mode select's value for "follow the workspace mode". */
const INHERIT = "inherit";

function ProfileForm({ agent }: { readonly agent: AssistantAgent }) {
  const { t } = useUiTranslation();
  const advancedControls = useFeature(FEATURE_FLAG.ASSISTANT_MODEL_CONTROLS);
  const update = useUpdateNyxBotAgent();
  const workspaceMode = useNyxBotSettings().data?.access_mode;
  const nyxbot = agent.kind === "nyxbot";
  const readOnly = agent.status === "destroyed" || agent.can_maintain === false;
  const form = useAppForm<AssistantAgentProfile>({
    resolver: zodResolver(assistantAgentProfileSchema(agent.kind)),
    defaultValues: {
      name: agent.name,
      display_name: agent.display_name ?? "",
      description: agent.description,
      persona: agent.persona ?? "",
      model: agent.model ?? "",
      access_mode_override: agent.access_mode_override ?? null,
    },
  });
  const [error, setError] = useState<string>();

  async function save(values: AssistantAgentProfile) {
    setError(undefined);
    const dirty = form.formState.dirtyFields;
    try {
      // Only what changed; an emptied display name or persona clears it.
      await update.mutateAsync({
        id: agent.id,
        ...(dirty.name && !nyxbot ? { name: values.name } : {}),
        ...(dirty.display_name ? { display_name: values.display_name } : {}),
        ...(dirty.description ? { description: values.description } : {}),
        ...(dirty.persona ? { persona: values.persona } : {}),
        ...(dirty.model ? { model: values.model } : {}),
        ...(dirty.access_mode_override ? { access_mode_override: values.access_mode_override } : {}),
      });
      form.reset(values);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save. Try again."));
    }
  }

  return (
    <Section title={t("Profile")}>
      <Form {...form}>
        <form aria-label={t("Profile")} noValidate onSubmit={form.handleSubmit(save)} className="space-y-3">
          {nyxbot ? null : (
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t("Name")}</FormLabel>
                  <FormControl>
                    <Input
                      maxLength={32}
                      disabled={readOnly}
                      {...field}
                      onChange={(event) => field.onChange(event.target.value.toLowerCase())}
                    />
                  </FormControl>
                  <FormDescription className="text-11">{t("The @handle used in groups and by NyxBot.")}</FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          )}
          <FormField
            control={form.control}
            name="display_name"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Display name")}</FormLabel>
                <FormControl>
                  <Input
                    maxLength={ASSISTANT_AGENT_DISPLAY_NAME_MAX}
                    disabled={readOnly}
                    autoComplete="off"
                    placeholder={nyxbot ? t("NyxBot") : agent.name}
                    {...field}
                  />
                </FormControl>
                <FormDescription className="text-11">{t("Shown instead of ")}{nyxbot ? t("NyxBot") : t("@{{name}}", { name: agent.name })}{t(". Leave empty to use it.")}</FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          {advancedControls && <FormField
            control={form.control}
            name="model"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Model")}</FormLabel>
                <ModelSelect
                  label={t("Agent model")}
                  value={field.value || undefined}
                  disabled={readOnly}
                  onChange={field.onChange}
                />
                <FormDescription className="text-11">{t("Used by this agent's new threads. Each thread can switch from its composer.")}</FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />}
          <FormField
            control={form.control}
            name="access_mode_override"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Access mode")}</FormLabel>
                <Select
                  value={field.value ?? INHERIT}
                  disabled={readOnly}
                  onValueChange={(value) =>
                    field.onChange(value === INHERIT ? null : nyxAgentAccessModeSchema.parse(value))
                  }
                >
                  <FormControl>
                    <SelectTrigger aria-label={t("Agent access mode")}>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value={INHERIT}>{t("Workspace setting")}{workspaceMode ? t(" ({{value}})", { value: ACCESS_MODE_LABEL[workspaceMode] }) : ""}
                    </SelectItem>
                    {nyxAgentAccessModeSchema.options.map((option) => (
                      <SelectItem key={option} value={option}>
                        {ACCESS_MODE_LABEL[option]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormDescription className="text-11">
                  {ACCESS_MODE_DESCRIPTION[effectiveAccessMode(workspaceMode, field.value)]}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="description"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{nyxbot ? t("Notes") : t("Role")}</FormLabel>
                <FormControl>
                  <textarea
                    className={TEXTAREA_CLASS}
                    maxLength={2048}
                    disabled={readOnly}
                    placeholder={nyxbot ? t("How NyxBot should work with you (optional).") : undefined}
                    {...field}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="persona"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Persona")}</FormLabel>
                <FormControl>
                  <textarea
                    className={TEXTAREA_CLASS}
                    maxLength={ASSISTANT_AGENT_PERSONA_MAX}
                    disabled={readOnly}
                    placeholder={PERSONA_HINT}
                    {...field}
                  />
                </FormControl>
                <FormDescription className="text-11">{t("Personality and tone only; it never changes what the agent may do.")}</FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          {readOnly ? null : (
            <div className="flex justify-end">
              <Button
                type="submit"
                size="sm"
                variant="primary"
                isLoading={update.isPending}
                disabled={!form.formState.isDirty}
              >{t("Save")}</Button>
            </div>
          )}
        </form>
      </Form>
    </Section>
  );
}

function GrantsForm({ agent }: { readonly agent: AssistantAgent }) {
  const { t } = useUiTranslation();
  const grants = useSetNyxBotAgentGrants();
  const readOnly = agent.status === "destroyed" || agent.can_maintain === false;
  const form = useAppForm<AssistantAgentGrants>({
    resolver: zodResolver(assistantAgentGrantsSchema),
    defaultValues: {
      services: agent.services,
      machines: agent.machines ?? [],
      logins: agent.logins ?? [],
      account_read: agent.account_read,
      guest_access: agent.guest_access,
    },
  });
  const [error, setError] = useState<string>();
  const services = form.watch("services");

  async function save(values: AssistantAgentGrants) {
    setError(undefined);
    // Send only the levels changed here, for services being granted: the
    // server keeps the others, and drops levels of services taken away.
    const changed = Object.fromEntries(
      values.services
        .map((slug) => [slug, values.guest_access[slug] ?? "use"] as const)
        .filter(([slug, level]) => level !== (agent.guest_access[slug] ?? "use")),
    );
    try {
      await grants.mutateAsync({
        id: agent.id,
        services: values.services,
        ...(form.getFieldState("machines").isDirty ? { machines: values.machines } : {}),
        ...(form.getFieldState("logins").isDirty ? { logins: values.logins } : {}),
        account_read: values.account_read,
        ...(Object.keys(changed).length ? { guest_access: changed } : {}),
      });
      form.reset(values);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save the grants. Try again."));
    }
  }

  return (
    <Section
      id={GRANTS_SECTION_ID}
      title={t("Grants")}
      description={t("This specialist is sandboxed and cannot grant access itself. Saving replaces the current grants on every thread. When you talk to it directly, anything beyond them is reported to you as missing; only work NyxBot assigned, or work in a group NyxBot is in, asks NyxBot.")}
    >
      <Form {...form}>
        <form aria-label={t("Grants")} noValidate onSubmit={form.handleSubmit(save)} className="space-y-3">
          <FormField
            control={form.control}
            name="services"
            render={({ field }) => (
              <FormItem>
                <ServiceGrantPicker
                  org={agent.owner_kind === "org" ? agent.owner_id : undefined}
                  value={field.value}
                  onChange={field.onChange}
                  disabled={readOnly}
                />
              </FormItem>
            )}
          />
          {(["machines", ...(agent.owner_kind === "org" ? [] : ["logins"])] as (
              | "machines"
              | "logins"
            )[]
          ).map((kind) => (
            <FormField key={kind} control={form.control} name={kind} render={({ field }) => (
                <FormItem>
                  <MachineGrantPicker
                    org={
                      agent.owner_kind === "org" ? agent.owner_id : undefined
                    }
                    kind={kind} value={field.value ?? []} onChange={field.onChange} disabled={readOnly} />
                </FormItem>
              )}
            />
          ))}
          {services.length && agent.owner_kind !== "org" ? (
            <FormField
              control={form.control}
              name="guest_access"
              render={({ field }) => (
                <FormItem>
                  <GuestAccessList
                    services={services}
                    value={field.value}
                    onChange={field.onChange}
                    disabled={readOnly}
                  />
                </FormItem>
              )}
            />
          ) : null}
          {agent.owner_kind !== "org" ? (
            <FormField
            control={form.control}
            name="account_read"
            render={({ field }) => (
              <FormItem>
                <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
                  <div className="space-y-1">
                    <FormLabel>{t("Read my account")}</FormLabel>
                    <FormDescription className="text-12">{t("Look up keys, services and nodes; never change them.")}</FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      disabled={readOnly}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </div>
              </FormItem>
            )}
          />
          ) : null}
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          {readOnly ? null : (
            <div className="flex justify-end">
              <Button
                type="submit"
                size="sm"
                variant="primary"
                isLoading={grants.isPending}
                disabled={!form.formState.isDirty}
              >{t("Save grants")}</Button>
            </div>
          )}
        </form>
      </Form>
      {agent.can_maintain !== false ? (
        <>
          <AgentOperationScopes agentId={agent.id} disabled={readOnly} />
          <MachineCapabilities agentId={agent.id} disabled={readOnly} />
        </>
      ) : null}
    </Section>
  );
}

const GUEST_ACCESS_LABEL: Record<AssistantGuestAccess, string> = {
  get read() { return translateUi("Look things up only"); },
  get use() { return translateUi("Use, but not change or delete"); },
  get all() { return translateUi("Everything this agent can do"); },
};

/**
 * What people other than you may do with each granted service when they talk
 * to this agent in a group or shared chat. Anything behind your approval
 * stays yours at every level.
 */
function GuestAccessList({
  services,
  value,
  onChange,
  disabled,
}: {
  readonly services: readonly string[];
  readonly value: Record<string, AssistantGuestAccess>;
  readonly onChange: (value: Record<string, AssistantGuestAccess>) => void;
  readonly disabled?: boolean;
}) {
  const { t } = useUiTranslation();
  return (
    <div className="space-y-2 rounded-lg border border-border p-4">
      <div className="space-y-1">
        <p className="text-12 font-medium text-foreground">{t("What others in its chats may do")}</p>
        <p className="text-12 text-muted-foreground">{t("For people other than you in the agent's group and shared chats. “Use” lets them look things up, create and act (send a message, turn a light on), but not change or delete what already exists. Anything behind your approval stays yours. You can also ask NyxBot to change this.")}</p>
      </div>
      <ul aria-label={t("Guest access")} className="space-y-1.5">
        {services.map((slug) => {
          const id = `guest-access-${slug}`;
          return (
            <li key={slug} className="flex items-center justify-between gap-3">
              <label
                htmlFor={id}
                className="min-w-0 flex-1 truncate font-mono text-11 text-muted-foreground"
              >
                {slug}
              </label>
              <Select
                value={value[slug] ?? "use"}
                disabled={disabled}
                onValueChange={(level) =>
                  onChange({ ...value, [slug]: level as AssistantGuestAccess })
                }
              >
                <SelectTrigger id={id} className="w-56">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(Object.keys(GUEST_ACCESS_LABEL) as AssistantGuestAccess[]).map((level) => (
                    <SelectItem key={level} value={level}>
                      {GUEST_ACCESS_LABEL[level]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function MemoryList({
  agent,
  memory,
}: {
  readonly agent: AssistantAgent;
  readonly memory: readonly AssistantAgentMemoryNote[];
}) {
  const { t } = useUiTranslation();
  const forget = useForgetNyxBotMemory();
  const [error, setError] = useState<string>();
  const name = agentTitle(agent);
  const remember = useRememberNyxBotMemory();
  const memoryForm = useAppForm<{ text: string }>({
    resolver: zodResolver(assistantMemoryNoteSchema),
    defaultValues: { text: "" },
  });
  const [editing, setEditing] = useState<string>();
  async function saveNote({ text }: { text: string }) {
    setError(undefined);
    try {
      await remember.mutateAsync({
        agentId: agent.id,
        text,
        replaceId: editing,
      });
      memoryForm.reset({ text: "" });
      setEditing(undefined);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save shared memory."));
    }
  }

  async function remove(note: AssistantAgentMemoryNote) {
    setError(undefined);
    try {
      await forget.mutateAsync({ agentId: agent.id, noteId: note.id });
    } catch (cause) {
      setError(errorMessage(cause, "Could not forget this note. Try again."));
    }
  }

  return (
    <Section
      title={t("Memory")}
      description={
        agent.owner_kind === "org"
          ? t("Shared organization memory. Never save private member content here.")
          : t("What {{name}} remembers across its threads and chat apps.", { name: name })}
    >
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {translateUi(error)}
        </p>
      ) : null}
      {agent.owner_kind === "org" && agent.status !== "destroyed" ? (
        <form
          aria-label={t("Shared memory")}
          onSubmit={memoryForm.handleSubmit(saveNote)}
          className="space-y-2"
        >
          <label
            htmlFor="shared-memory-note"
            className="text-12 font-medium"
          >
            {editing ? t("Edit shared note") : t("Add shared note")}
          </label>
          <textarea
            id="shared-memory-note"
            className={TEXTAREA_CLASS}
            maxLength={ASSISTANT_MEMORY_NOTE_MAX}
            {...memoryForm.register("text")}
          />
          <div className="flex justify-end gap-2">
            {editing ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={() => {
                  setEditing(undefined);
                  memoryForm.reset({ text: "" });
                }}
              >{t("Cancel edit")}</Button>
            ) : null}
            <Button
              type="submit"
              size="sm"
              variant="primary"
              isLoading={remember.isPending}
              disabled={
                !memoryForm.formState.isDirty ||
                !memoryForm.watch("text").trim()
              }
            >{t("Save shared note")}</Button>
          </div>
        </form>
      ) : null}
      {memory.length ? (
        <ul aria-label={t("{{name}} memory", { name: name })} className="divide-y divide-border/30 rounded-lg border border-border">
          {memory.map((note) => {
return (
            <li key={note.id} className="flex items-start gap-2 px-3 py-2.5">
              <div className="min-w-0 flex-1 space-y-0.5">
                <p className="whitespace-pre-wrap break-words text-12 text-foreground">
                  {note.text}
                </p>
                <p className="font-mono text-10 text-text-tertiary">
                  {formatDateTime(note.updated_at)}
                </p>
              </div>
              {agent.owner_kind === "org" && agent.status !== "destroyed" ? (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setEditing(note.id);
                    memoryForm.reset({ text: note.text });
                  }}
                >{t("Edit note")}</Button>
              ) : null}
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 shrink-0"
                aria-label={t("Forget: {{value}}", { value: note.text.slice(0, 60) })}
                disabled={forget.isPending}
                onClick={() => void remove(note)}
              >
                <Trash2 />
              </Button>
            </li>
          );
})}
        </ul>
      ) : (
        <p className="rounded-lg border border-dashed border-border px-4 py-3 text-12 text-muted-foreground">{t("Nothing remembered yet.")}</p>
      )}
    </Section>
  );
}

function Lifecycle({
  agent,
  onDeleted,
}: {
  readonly agent: AssistantAgent;
  readonly onDeleted: () => void;
}) {
  const { t } = useUiTranslation();
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const destroy = useDestroyNyxBotAgent();
  const remove = useDeleteNyxBotAgent();
  const [confirm, setConfirm] = useState<"destroy" | "delete">();
  const [error, setError] = useState<string>();
  const destroyed = agent.status === "destroyed";
  const pending = destroy.isPending || remove.isPending;

  async function run() {
    setError(undefined);
    try {
      if (confirm === "destroy") {
        await destroy.mutateAsync(agent.id);
        // Its open threads become read-only.
        await queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.root(userId) });
      } else {
        await remove.mutateAsync(agent.id);
        nyxAgentTransport.forgetAgent(agent.id);
        onDeleted();
      }
      setConfirm(undefined);
    } catch (cause) {
      setError(errorMessage(cause, "That did not work. Try again."));
    }
  }

  return (
    <section
      aria-label={t("Danger zone")}
      className="space-y-3 rounded-xl border border-destructive/40 p-4"
    >
      <div className="space-y-1">
        <h3 className="text-13 font-semibold text-destructive">
          {destroyed ? t("Delete permanently") : t("Destroy agent")}
        </h3>
        <p className="text-12 text-destructive/70">
          {destroyed
            ? t("{{value}} was destroyed{{value1}}. Deleting removes it and all of its threads for good.", { value: agentTitle(agent), value1: agent.destroyed_at ? ` on ${formatDateTime(agent.destroyed_at)}` : "" })
            : t("Stops its work, revokes its access and disconnects its channel bots. Its threads stay readable.")}
        </p>
      </div>
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="destructive"
          onClick={() => {
            setError(undefined);
            setConfirm(destroyed ? "delete" : "destroy");
          }}
        >
          {destroyed ? t("Delete permanently") : t("Destroy")}
        </Button>
      </div>
      <Dialog
        open={confirm !== undefined}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirm(undefined);
        }}
      >
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {confirm === "delete"
                ? t("Delete {{value}} permanently?", { value: agentTitle(agent) })
                : t("Destroy {{value}}?", { value: agentTitle(agent) })}
            </DialogTitle>
            <DialogDescription>
              {confirm === "delete"
                ? t("The agent, its memory and every thread with it are deleted. This cannot be undone.")
                : t("Any running turn stops, its access is revoked and its channel bots are disconnected. Its threads stay here, read-only. This cannot be undone.")}
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          <DialogFooter>
            <Button variant="ghost" disabled={pending} onClick={() => setConfirm(undefined)}>{t("Cancel")}</Button>
            <Button variant="destructive" isLoading={pending} onClick={() => void run()}>
              {confirm === "delete" ? t("Delete permanently") : t("Destroy")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

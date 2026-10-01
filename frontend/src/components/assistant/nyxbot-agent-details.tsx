import { MachineGrantPicker } from "./machine-grant-picker";
import { AgentAutomations } from "./automation-preferences";
import { useState, type ReactNode } from "react";
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
  useNyxBotAgent,
  useSetNyxBotAgentGrants,
  useUpdateNyxBotAgent,
} from "@/hooks/use-nyxbot-agents";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { AGENT_STATUS_LABEL, agentHandle, agentTitle } from "@/lib/assistant/nyxbot-labels";
import { formatDateTime } from "@/lib/utils";
import {
  ASSISTANT_AGENT_DISPLAY_NAME_MAX,
  ASSISTANT_AGENT_PERSONA_MAX,
  assistantAgentGrantsSchema,
  assistantAgentProfileSchema,
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
}: {
  readonly title: string;
  readonly description?: string;
  readonly children?: ReactNode;
}) {
  return (
    <section aria-label={title} className="space-y-3">
      <div className="space-y-1">
        <h3 className="text-[13px] font-semibold text-foreground">{title}</h3>
        {description ? <p className="text-[12px] text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </section>
  );
}

/**
 * Everything about one agent: its role, grants, memory, channel bots and
 * lifecycle. Opened from the thread header.
 */
export function AgentDetailsSheet({
  agentId,
  agents,
  open,
  onOpenChange,
  onDeleted,
}: {
  readonly agentId: string | undefined;
  readonly agents: readonly AssistantAgent[];
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onDeleted: () => void;
}) {
  const detail = useNyxBotAgent(agentId, open);
  const agent = detail.data?.agent;
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader className="space-y-1.5 pr-6">
          <SheetTitle>Agent details</SheetTitle>
          <SheetDescription>
            {agent
              ? agent.kind === "nyxbot"
                ? "Your personal agent. It has full access and remembers across all its threads and chat apps."
                : "A specialist with its own memory. It uses only its grants and asks NyxBot for anything else."
              : "Loading agent..."}
          </SheetDescription>
        </SheetHeader>
        <div className="mt-5 space-y-8">
          {detail.error ? (
            <ErrorBanner
              message={`Could not load this agent. ${detail.error.message}`}
              onRetry={() => void detail.refetch()}
            />
          ) : null}
          {agent ? (
            <>
              <div className="flex flex-wrap items-center gap-2 text-[11px] text-text-tertiary">
                <AgentAvatar agent={agent} size="lg" />
                <span className="text-[15px] font-semibold text-foreground">{agentTitle(agent)}</span>
                {agentHandle(agent) ? <span>{agentHandle(agent)}</span> : null}
                <AgentKindBadge kind={agent.kind} />
                <Badge variant={agent.status === "running" ? "success" : "secondary"}>
                  {AGENT_STATUS_LABEL[agent.status]}
                </Badge>
                <span>
                  {agent.kind === "specialist"
                    ? `Created by ${agent.created_by === "nyxbot" ? "NyxBot" : "you"} · `
                    : ""}
                  {agent.memory_count} {agent.memory_count === 1 ? "memory" : "memories"}
                </span>
              </div>
              <ProfileForm
                key={[agent.id, agent.name, agent.description, agent.display_name, agent.persona].join(
                  ":",
                )}
                agent={agent}
              />
              {agent.kind === "specialist" ? (
                <GrantsForm
                  key={`${agent.id}:${agent.services.join(",")}:${String(agent.account_read)}:${JSON.stringify(agent.guest_access)}`}
                  agent={agent}
                />
              ) : (
                <Section
                  title="Access"
                  description="NyxBot runs with full access to your connected services and account. Destructive actions follow your confirmation setting."
                />
              )}
              <AgentAutomations agentId={agent.id} />
              <MemoryList agent={agent} memory={detail.data?.memory ?? []} />
              {agent.status === "destroyed" ? null : (
                <Section
                  title="Channel bots"
                  description={`Chat with ${agentTitle(agent)} from these bots.`}
                >
                  <ChannelBotsManager agents={agents} agent={agent} />
                </Section>
              )}
              {agent.kind === "specialist" ? (
                <Lifecycle agent={agent} onDeleted={onDeleted} />
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function ProfileForm({ agent }: { readonly agent: AssistantAgent }) {
  const update = useUpdateNyxBotAgent();
  const nyxbot = agent.kind === "nyxbot";
  const readOnly = agent.status === "destroyed";
  const form = useAppForm<AssistantAgentProfile>({
    resolver: zodResolver(assistantAgentProfileSchema(agent.kind)),
    defaultValues: {
      name: agent.name,
      display_name: agent.display_name ?? "",
      description: agent.description,
      persona: agent.persona ?? "",
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
      });
      form.reset(values);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save. Try again."));
    }
  }

  return (
    <Section title="Profile">
      <Form {...form}>
        <form aria-label="Profile" noValidate onSubmit={form.handleSubmit(save)} className="space-y-3">
          {nyxbot ? null : (
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Name</FormLabel>
                  <FormControl>
                    <Input
                      maxLength={32}
                      disabled={readOnly}
                      {...field}
                      onChange={(event) => field.onChange(event.target.value.toLowerCase())}
                    />
                  </FormControl>
                  <FormDescription className="text-[11px]">
                    The @handle used in groups and by NyxBot.
                  </FormDescription>
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
                <FormLabel>Display name</FormLabel>
                <FormControl>
                  <Input
                    maxLength={ASSISTANT_AGENT_DISPLAY_NAME_MAX}
                    disabled={readOnly}
                    autoComplete="off"
                    placeholder={nyxbot ? "NyxBot" : agent.name}
                    {...field}
                  />
                </FormControl>
                <FormDescription className="text-[11px]">
                  Shown instead of {nyxbot ? "NyxBot" : `@${agent.name}`}. Leave empty to use it.
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
                <FormLabel>{nyxbot ? "Notes" : "Role"}</FormLabel>
                <FormControl>
                  <textarea
                    className={TEXTAREA_CLASS}
                    maxLength={2048}
                    disabled={readOnly}
                    placeholder={nyxbot ? "How NyxBot should work with you (optional)." : undefined}
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
                <FormLabel>Persona</FormLabel>
                <FormControl>
                  <textarea
                    className={TEXTAREA_CLASS}
                    maxLength={ASSISTANT_AGENT_PERSONA_MAX}
                    disabled={readOnly}
                    placeholder={PERSONA_HINT}
                    {...field}
                  />
                </FormControl>
                <FormDescription className="text-[11px]">
                  Personality and tone only; it never changes what the agent may do.
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          {error ? (
            <p role="alert" className="text-[12px] text-destructive">
              {error}
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
              >
                Save
              </Button>
            </div>
          )}
        </form>
      </Form>
    </Section>
  );
}

function GrantsForm({ agent }: { readonly agent: AssistantAgent }) {
  const grants = useSetNyxBotAgentGrants();
  const readOnly = agent.status === "destroyed";
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
      title="Grants"
      description="Saving replaces the current grants. Anything else, the agent asks NyxBot for."
    >
      <Form {...form}>
        <form aria-label="Grants" noValidate onSubmit={form.handleSubmit(save)} className="space-y-3">
          <FormField
            control={form.control}
            name="services"
            render={({ field }) => (
              <FormItem>
                <ServiceGrantPicker
                  value={field.value}
                  onChange={field.onChange}
                  disabled={readOnly}
                />
              </FormItem>
            )}
          />
          {(["machines", "logins"] as const).map((kind) => <FormField key={kind} control={form.control} name={kind} render={({ field }) => <FormItem><MachineGrantPicker kind={kind} value={field.value ?? []} onChange={field.onChange} disabled={readOnly} /></FormItem>} />)}
          {services.length ? (
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
          <FormField
            control={form.control}
            name="account_read"
            render={({ field }) => (
              <FormItem>
                <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
                  <div className="space-y-1">
                    <FormLabel>Read my account</FormLabel>
                    <FormDescription className="text-[12px]">
                      Look up keys, services and nodes; never change them.
                    </FormDescription>
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
          {error ? (
            <p role="alert" className="text-[12px] text-destructive">
              {error}
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
              >
                Save grants
              </Button>
            </div>
          )}
        </form>
      </Form>
    </Section>
  );
}

const GUEST_ACCESS_LABEL: Record<AssistantGuestAccess, string> = {
  read: "Look things up only",
  use: "Use, but not change or delete",
  all: "Everything this agent can do",
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
  return (
    <div className="space-y-2 rounded-lg border border-border p-4">
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-foreground">
          What others in its chats may do
        </p>
        <p className="text-[12px] text-muted-foreground">
          For people other than you in the agent&apos;s group and shared chats. &ldquo;Use&rdquo;
          lets them look things up, create and act (send a message, turn a light on), but not
          change or delete what already exists. Anything behind your approval stays yours. You
          can also ask NyxBot to change this.
        </p>
      </div>
      <ul aria-label="Guest access" className="space-y-1.5">
        {services.map((slug) => {
          const id = `guest-access-${slug}`;
          return (
            <li key={slug} className="flex items-center justify-between gap-3">
              <label
                htmlFor={id}
                className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground"
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
  const forget = useForgetNyxBotMemory();
  const [error, setError] = useState<string>();
  const name = agentTitle(agent);

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
      title="Memory"
      description={`What ${name} remembers across its threads and chat apps.`}
    >
      {error ? (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      ) : null}
      {memory.length ? (
        <ul aria-label={`${name} memory`} className="divide-y divide-border/30 rounded-lg border border-border">
          {memory.map((note) => (
            <li key={note.id} className="flex items-start gap-2 px-3 py-2.5">
              <div className="min-w-0 flex-1 space-y-0.5">
                <p className="whitespace-pre-wrap break-words text-[12px] text-foreground">
                  {note.text}
                </p>
                <p className="font-mono text-[10px] text-text-tertiary">
                  {formatDateTime(note.updated_at)}
                </p>
              </div>
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 shrink-0"
                aria-label={`Forget: ${note.text.slice(0, 60)}`}
                disabled={forget.isPending}
                onClick={() => void remove(note)}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="rounded-lg bg-overlay px-4 py-3 text-[12px] text-muted-foreground">
          Nothing remembered yet.
        </p>
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
      aria-label="Danger zone"
      className="space-y-3 rounded-xl border border-destructive/40 p-4"
    >
      <div className="space-y-1">
        <h3 className="text-[13px] font-semibold text-destructive">
          {destroyed ? "Delete permanently" : "Destroy agent"}
        </h3>
        <p className="text-[12px] text-destructive/70">
          {destroyed
            ? `${agentTitle(agent)} was destroyed${agent.destroyed_at ? ` on ${formatDateTime(agent.destroyed_at)}` : ""}. Deleting removes it and all of its threads for good.`
            : "Stops its work, revokes its access and disconnects its channel bots. Its threads stay readable."}
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
          {destroyed ? "Delete permanently" : "Destroy"}
        </Button>
      </div>
      <Dialog
        open={confirm !== undefined}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirm(undefined);
        }}
      >
        <DialogContent className="z-[90] md:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {confirm === "delete"
                ? `Delete ${agentTitle(agent)} permanently?`
                : `Destroy ${agentTitle(agent)}?`}
            </DialogTitle>
            <DialogDescription>
              {confirm === "delete"
                ? "The agent, its memory and every thread with it are deleted. This cannot be undone."
                : "Any running turn stops, its access is revoked and its channel bots are disconnected. Its threads stay here, read-only. This cannot be undone."}
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-[12px] text-destructive">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button variant="ghost" disabled={pending} onClick={() => setConfirm(undefined)}>
              Cancel
            </Button>
            <Button variant="destructive" isLoading={pending} onClick={() => void run()}>
              {confirm === "delete" ? "Delete permanently" : "Destroy"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

import { useState } from "react";
import { useOrgs } from "@/hooks/use-orgs";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { zodResolver } from "@hookform/resolvers/zod";
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
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { MachineGrantPicker } from "./machine-grant-picker";
import { useKeys } from "@/hooks/use-keys";
import { useCreateNyxBotAgent } from "@/hooks/use-nyxbot-agents";
import {
  ASSISTANT_AGENT_DISPLAY_NAME_MAX,
  ASSISTANT_AGENT_PERSONA_MAX,
  assistantAgentCreateSchema,
  type AssistantAgentCreate,
} from "@/schemas/assistant-nyxagent";

export const PERSONA_HINT = "e.g. warm, concise, uses emoji sparingly";

export const TEXTAREA_CLASS =
  "min-h-20 w-full rounded-lg border border-input bg-transparent px-3 py-1.5 text-[12px] leading-relaxed text-foreground placeholder:text-text-tertiary focus-visible:border-white/[0.15] focus-visible:outline-none aria-invalid:border-destructive";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

/**
 * Pick the connected services an agent may use, by slug. Slugs already
 * granted but no longer connected stay listed so they can be removed.
 */
export function ServiceGrantPicker({
  value,
  onChange,
  disabled,
  org,
}: {
  readonly org?: string;
  readonly value: readonly string[];
  readonly onChange: (slugs: string[]) => void;
  readonly disabled?: boolean;
}) {
  const keys = useKeys();
  const services = (keys.data ?? [])
    .filter((key) => key.is_active &&
        (!org ||
          (key.credential_source?.type === "org" &&
            key.credential_source.org_id === org &&
            key.credential_source.allowed)),
    )
    .map((key) => ({ slug: org ? key.id : key.slug, label: key.label }));
  const known = new Set(services.map((service) => service.slug));
  const rows = [
    ...services,
    ...value.filter((slug) => !known.has(slug)).map((slug) => ({ slug, label: slug })),
  ];
  function toggle(slug: string, checked: boolean) {
    onChange(checked ? [...value, slug] : value.filter((item) => item !== slug));
  }
  if (keys.isPending) {
    return (
      <p className="text-[12px] text-text-tertiary">Loading your services...</p>
    );
  }
  if (!rows.length) {
    return (
      <p className="rounded-lg bg-overlay px-3 py-2 text-[12px] text-muted-foreground">
        You have no connected services yet. The agent can ask NyxBot for access later.
      </p>
    );
  }
  return (
    <ul
      aria-label="Services"
      className="assistant-scrollbar max-h-44 space-y-0.5 overflow-y-auto rounded-lg border border-border p-1"
    >
      {rows.map((service) => {
        const id = `grant-${service.slug}`;
        return (
          <li key={service.slug}>
            <label
              htmlFor={id}
              className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 text-[12px] hover:bg-overlay"
            >
              <Checkbox
                id={id}
                checked={value.includes(service.slug)}
                disabled={disabled}
                onCheckedChange={(checked) => toggle(service.slug, checked === true)}
              />
              <span className="min-w-0 flex-1 truncate text-foreground">
                {service.label}
              </span>
              <span className="shrink-0 font-mono text-[10px] text-text-tertiary">
                {service.slug}
              </span>
            </label>
          </li>
        );
      })}
    </ul>
  );
}

function AgentOwnerPicker({
  value,
  onChange,
}: {
  readonly value?: string;
  readonly onChange: (id: string | undefined) => void;
}) {
  const orgs = useOrgs();
  const writable = (orgs.data ?? []).filter((org) =>
    ["admin", "member", "owner"].includes(org.your_role),
  );
  return (
    <div className="space-y-2">
      <label className="text-[12px] font-medium" htmlFor="agent-owner">
        Ownership
      </label>
      <Select
        value={value ?? "personal"}
        onValueChange={(id) => onChange(id === "personal" ? undefined : id)}
      >
        <SelectTrigger id="agent-owner">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="personal">Personal</SelectItem>
          {writable.map((org) => (
            <SelectItem key={org.id} value={org.id}>
              {org.display_name ?? org.slug}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {value ? (
        <p className="text-[11px] text-muted-foreground">
          Admins and Members maintain this agent. Each member has private chats;
          memory is shared.
        </p>
      ) : null}
    </div>
  );
}

/** Create a specialist: its name, role and starting grants. */
export function NewAgentDialog({
  onClose,
  onCreated,
}: {
  readonly onClose: () => void;
  readonly onCreated: (created: { id: string; home_conversation_id: string }) => void;
}) {
  const create = useCreateNyxBotAgent();
  const orgAgentsEnabled = useFeature(FEATURE_FLAG.ORG_AGENTS);
  const form = useAppForm<AssistantAgentCreate>({
    resolver: zodResolver(assistantAgentCreateSchema),
    defaultValues: {
      name: "",
      display_name: "",
      description: "",
      persona: "",
      services: [],
      account_read: false,
    },
  });
  const [error, setError] = useState<string>();
  const name = form.watch("name");
  const description = form.watch("description");
  const org = form.watch("org");

  async function submit(values: AssistantAgentCreate) {
    setError(undefined);
    try {
      const created = await create.mutateAsync(values);
      onCreated(created);
    } catch (cause) {
      setError(errorMessage(cause, "Could not create the agent. Try again."));
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !create.isPending) onClose();
      }}
    >
      <DialogContent scrollMode="body" className="z-[90] md:max-w-lg">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>New agent</DialogTitle>
          <DialogDescription>
            A specialist keeps its own memory and threads, uses only the
            services you grant here, and asks NyxBot for anything else.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form noValidate onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-4 pb-1">
              {orgAgentsEnabled ? (
                <AgentOwnerPicker
                  value={org}
                  onChange={(id) => {
                    form.setValue("org", id);
                    form.setValue("services", []);
                    form.setValue("machines", []);
                    form.setValue("logins", []);
                    form.setValue("account_read", false);
                  }}
                />
              ) : (
                <p className="text-[11px] text-text-tertiary">
                  Organization agents are not enabled yet.
                </p>
              )}
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="researcher"
                        autoComplete="off"
                        maxLength={32}
                        {...field}
                        onChange={(event) => field.onChange(event.target.value.toLowerCase())}
                      />
                    </FormControl>
                    <FormDescription className="text-[11px]">
                      The @handle: lowercase letters, digits and hyphens.
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="display_name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      Display name{" "}
                      <span className="font-normal text-text-tertiary">
                        (optional)
                      </span>
                    </FormLabel>
                    <FormControl>
                      <Input
                        placeholder="Luna"
                        autoComplete="off"
                        maxLength={ASSISTANT_AGENT_DISPLAY_NAME_MAX}
                        {...field}
                      />
                    </FormControl>
                    <FormDescription className="text-[11px]">
                      Shown instead of the @handle.
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
                    <FormLabel>Role</FormLabel>
                    <FormControl>
                      <textarea
                        className={TEXTAREA_CLASS}
                        maxLength={2048}
                        placeholder="Tracks open GitHub issues and reports the urgent ones."
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
                    <FormLabel>
                      Persona{" "}
                      <span className="font-normal text-text-tertiary">
                        (optional)
                      </span>
                    </FormLabel>
                    <FormControl>
                      <textarea
                        className={TEXTAREA_CLASS}
                        maxLength={ASSISTANT_AGENT_PERSONA_MAX}
                        placeholder={PERSONA_HINT}
                        {...field}
                      />
                    </FormControl>
                    <FormDescription className="text-[11px]">
                      Personality and tone only; it never changes what the agent
                      may do.
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="services"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Services it may use</FormLabel>
                    <ServiceGrantPicker
                      org={org}
                      value={field.value} onChange={field.onChange} />
                    <FormMessage />
                  </FormItem>
                )}
              />
              {(["machines", ...(org ? [] : ["logins"])] as (
                  | "machines"
                  | "logins"
                )[]
              ).map((kind) => (
                <FormField key={kind} control={form.control} name={kind} render={({ field }) => (
                  <FormItem>
                      <MachineGrantPicker
                        org={org}
                        kind={kind} value={field.value ?? []} onChange={field.onChange} disabled={create.isPending} />
                      <FormMessage />
                    </FormItem>
                )} />
              ))}
              {!org ? (
                <FormField
                control={form.control}
                name="account_read"
                render={({ field }) => (
                  <FormItem>
                    <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
                      <div className="space-y-1">
                        <FormLabel>Read my account</FormLabel>
                        <FormDescription className="text-[12px]">
                            Lets it look up your keys, services and nodes. It
                            can never change or delete them.
                          </FormDescription>
                      </div>
                      <FormControl>
                        <Switch checked={field.value} onCheckedChange={field.onChange} />
                      </FormControl>
                    </div>
                  </FormItem>
                )}
              />
              ) : null}
              {error ? (
                <p role="alert" className="text-[12px] text-destructive">
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
                disabled={!name.trim() || !description.trim()}
              >
                Create agent
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

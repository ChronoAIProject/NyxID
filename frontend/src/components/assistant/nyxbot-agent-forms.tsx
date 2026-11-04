import { useDrawerSearchEntries } from "@/lib/drawer-search";
import { flushSync } from "react-dom";
import { useEffect, useState } from "react";
import { useOrgs } from "@/hooks/use-orgs";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { zodResolver } from "@hookform/resolvers/zod";
import { Check } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import {
  DialogBody,
  DialogDescription,
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
import {
  GroupedSelectTrigger as SelectTrigger,
  GroupedTextarea,
} from "@/components/ui/grouped-list";
import {
  GroupedField,
  GroupedLabel,
  GroupedDescription,
  GroupedMessage,
} from "@/components/ui/grouped-fields";
import {
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
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
  "min-h-20 w-full rounded-lg border border-input bg-transparent px-3 py-1.5 text-14 leading-relaxed text-foreground placeholder:text-text-tertiary focus-visible:border-input-focus focus-visible:outline-none aria-invalid:border-destructive";

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
  footer,
  org,
}: {
  readonly org?: string;
  readonly value: readonly string[];
  readonly onChange: (slugs: string[]) => void;
  readonly disabled?: boolean;
  readonly footer?: React.ReactNode;
}) {
  const keys = useKeys({ includeTools: true });
  const [search, setSearch] = useState("");
  const inventory = (keys.data ?? [])
    .filter(
      (key) =>
        key.is_active &&
        (!org ||
          (key.credential_source?.type === "org" &&
            key.credential_source.org_id === org &&
            key.credential_source.allowed)),
    )
    .map((key) => ({ slug: org ? key.id : key.slug, label: key.label }));
  const services = [
    ...new Map(inventory.map((service) => [service.slug, service])).values(),
  ];
  const known = new Set(services.map((service) => service.slug));
  const rows = [
    ...services,
    ...value.filter((slug) => !known.has(slug)).map((slug) => ({ slug, label: slug })),
  ];
  useDrawerSearchEntries(keys.isPending ? [] : rows.map((service) => ({
    id: `service-${service.slug}`, title: service.label, breadcrumb: `Access › Services › ${service.slug}`,
    keywords: `${service.slug} ${value.includes(service.slug) ? "Granted" : "Available"}`,
    reveal: () => setSearch(""),
  })));
  function toggle(slug: string, checked: boolean) {
    onChange(checked ? [...value, slug] : value.filter((item) => item !== slug));
  }
  if (keys.isPending) {
    return (
      <GroupedList title="Services" footer={footer}>
        <GroupedRow title="Loading your services..." />
      </GroupedList>
    );
  }
  if (!rows.length) {
    return (
      <GroupedList title="Services" footer={
        <>
          {footer}
          <p>
            You have no connected services yet. Connect a service first, then
            grant it here; the agent uses only what you grant.
          </p>
        </>
      }>
        <GroupedRow title="No connected services" />
      </GroupedList>
    );
  }
  return (
    <GroupedList
      title="Services"
      footer={
        <>
          {footer}
          {keys.error ? (
            <span role="alert" className="text-destructive">
              Could not refresh services. Your selections are kept.
            </span>
          ) : null}
        </>
      }
    >
      {rows.length > 8 ? (
        <GroupedRow>
          <GroupedInput
            aria-label="Search services"
            placeholder="Search services"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </GroupedRow>
      ) : null}
      <div
        role="group"
        aria-label="Services"
        className={
          rows.length > 12
            ? "assistant-scrollbar max-h-80 overflow-y-auto"
            : undefined
        }
      >
        {rows
          .filter((service) =>
            `${service.label} ${service.slug}`
              .toLowerCase()
              .includes(search.toLowerCase()),
          )
          .map((service) => (
            <GroupedRow data-drawer-search-target={`service-${service.slug}`} key={service.slug} asChild interactive>
              <button
                type="button"
                role="checkbox"
                aria-checked={value.includes(service.slug)}
                disabled={disabled}
                onClick={() =>
                  toggle(service.slug, !value.includes(service.slug))
                }
              >
                <span className="min-w-0 flex-1 break-words text-foreground">
                  {service.label}
                  <span className="mt-0.5 block font-mono text-13 font-normal text-text-tertiary">
                    {service.slug}
                  </span>
                </span>
                <Check
                  aria-hidden
                  className={
                    value.includes(service.slug)
                      ? "size-[1.125em] shrink-0 text-primary"
                      : "invisible size-[1.125em] shrink-0"
                  }
                />
              </button>
            </GroupedRow>
          ))}
        {search &&
        !rows.some((service) =>
          `${service.label} ${service.slug}`
            .toLowerCase()
            .includes(search.toLowerCase()),
        ) ? (
          <GroupedRow title="No matching services" />
        ) : null}
      </div>
    </GroupedList>
  );
}

/** Organizations the person may create agents for. */
function useWritableOrgs() {
  const orgs = useOrgs();
  return (orgs.data ?? []).filter((org) =>
    ["admin", "member", "owner"].includes(org.your_role),
  );
}

function AgentOwnerPicker({
  value,
  onChange,
}: {
  readonly value?: string;
  readonly onChange: (id: string | undefined) => void;
}) {
  const writable = useWritableOrgs();
  return (
    <GroupedList
      footer={
        value
          ? "Admins and Members maintain this agent. Each member has private chats; memory is shared."
          : undefined
      }
    >
      <GroupedRow>
        <label className="flex-1 text-14 font-medium" htmlFor="agent-owner">
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
      </GroupedRow>
    </GroupedList>
  );
}

/** Create a specialist: its name, role and starting grants. */
export function NewAgentDialog({
  owner = "personal",
  onClose,
  onCreated,
}: {
  /** "org" starts on the first organization the person can create agents for. */
  readonly owner?: "personal" | "org";
  readonly onClose: () => void;
  readonly onCreated: (created: {
    id: string;
    home_conversation_id: string;
  }) => void;
}) {
  const create = useCreateNyxBotAgent();
  const orgAgentsEnabled = useFeature(FEATURE_FLAG.ORG_AGENTS);
  const form = useAppForm<AssistantAgentCreate>({
    mode: "onChange",
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
  const org = form.watch("org");
  const firstOrg = useWritableOrgs()[0]?.id;
  // Organizations may still be loading when the dialog opens; preselect once known.
  useEffect(() => {
    if (owner !== "org" || !orgAgentsEnabled || !firstOrg || form.getValues("org")) return;
    form.setValue("org", firstOrg, { shouldDirty: false, shouldTouch: false });
  }, [firstOrg, form, orgAgentsEnabled, owner]);

  async function submit(values: AssistantAgentCreate) {
    setError(undefined);
    try {
      const created = await create.mutateAsync(values);
      flushSync(() => form.reset(values));
      onCreated(created);
    } catch (cause) {
      setError(errorMessage(cause, "Could not create the agent. Try again."));
    }
  }

  return (
    <GroupedDialog
      open
      onOpenChange={(open) => {
        if (!open && !create.isPending) onClose();
      }}
    >
      <GroupedDialogContent scrollMode="body" className="md:max-w-lg">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>New agent</DialogTitle>
          <DialogDescription>
            A specialist keeps its own memory and threads and uses only the
            services you grant here. When you talk to it directly it handles
            the work itself and tells you what it is missing; you add grants
            in its details.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form noValidate onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-6 pb-4">
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
                <GroupedList title="Ownership">
                  <GroupedRow title="Personal" />
                  <GroupedFooter>
                    Organization agents are not enabled yet.
                  </GroupedFooter>
                </GroupedList>
              )}
              <GroupedList title="Profile">
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <GroupedField>
                      <GroupedLabel>Name</GroupedLabel>
                      <FormControl>
                        <GroupedInput
                          placeholder="researcher"
                          autoComplete="off"
                          maxLength={32}
                          {...field}
                          onChange={(event) => field.onChange(event.target.value.toLowerCase())
                          }
                        />
                      </FormControl>
                      <GroupedDescription className="text-13">
                        The @handle: lowercase letters, digits and hyphens.
                      </GroupedDescription>
                      <GroupedMessage />
                    </GroupedField>
                  )}
                />
                <FormField
                  control={form.control}
                  name="display_name"
                  render={({ field }) => (
                    <GroupedField>
                      <GroupedLabel>
                        Display name{" "}
                        <span className="font-normal text-text-tertiary">
                          (optional)
                        </span>
                      </GroupedLabel>
                      <FormControl>
                        <GroupedInput
                          placeholder="Luna"
                          autoComplete="off"
                          maxLength={ASSISTANT_AGENT_DISPLAY_NAME_MAX}
                          {...field}
                        />
                      </FormControl>
                      <GroupedDescription className="text-13">
                        Shown instead of the @handle.
                      </GroupedDescription>
                      <GroupedMessage />
                    </GroupedField>
                  )}
                />
                <FormField
                  control={form.control}
                  name="description"
                  render={({ field }) => (
                    <GroupedField>
                      <GroupedLabel>Role</GroupedLabel>
                      <FormControl>
                        <GroupedTextarea
                          maxLength={2048}
                          placeholder="Tracks open GitHub issues and reports the urgent ones."
                          {...field}
                        />
                      </FormControl>
                      <GroupedMessage />
                    </GroupedField>
                  )}
                />
                <FormField
                  control={form.control}
                  name="persona"
                  render={({ field }) => (
                    <GroupedField>
                      <GroupedLabel>
                        Persona{" "}
                        <span className="font-normal text-text-tertiary">
                          (optional)
                        </span>
                      </GroupedLabel>
                      <FormControl>
                        <GroupedTextarea
                          maxLength={ASSISTANT_AGENT_PERSONA_MAX}
                          placeholder={PERSONA_HINT}
                          {...field}
                        />
                      </FormControl>
                      <GroupedDescription className="text-13">
                        Personality and tone only; it never changes what the
                        agent may do.
                      </GroupedDescription>
                      <GroupedMessage />
                    </GroupedField>
                  )}
                />
              </GroupedList>
              <FormField
                control={form.control}
                name="services"
                render={({ field }) => (
                  <FormItem>
                    <ServiceGrantPicker
                      footer={<FormMessage className="text-13" />}
                      org={org}
                      value={field.value}
                      onChange={field.onChange}
                    />
                  </FormItem>
                )}
              />
              {(
                ["machines", ...(org ? [] : ["logins"])] as (
                  | "machines"
                  | "logins"
                )[]
              ).map((kind) => (
                <FormField
                  key={kind}
                  control={form.control}
                  name={kind}
                  render={({ field }) => (
                    <FormItem>
                      <MachineGrantPicker
                        org={org}
                        kind={kind}
                        value={field.value ?? []}
                        onChange={field.onChange}
                        disabled={create.isPending}
                        footer={<FormMessage className="text-13" />}
                      />
                    </FormItem>
                  )}
                />
              ))}
              {!org ? (
                <FormField
                  control={form.control}
                  name="account_read"
                  render={({ field }) => (
                    <GroupedList title="Account access">
                      <GroupedField inline>
                        <div className="min-w-0 flex-1">
                          <FormLabel className="text-14 font-medium">
                            Read my account
                          </FormLabel>
                          <GroupedDescription>
                            Lets it look up your keys, services and nodes. It
                            can never change or delete them.
                          </GroupedDescription>
                        </div>
                        <FormControl>
                          <Switch checked={field.value} onCheckedChange={field.onChange} />
                        </FormControl>
                      </GroupedField>
                    </GroupedList>
                  )}
                />
              ) : null}
              {error ? (
                <GroupedList
                  footer={
                    <p role="alert" className="text-destructive">
                      {error}
                    </p>
                  }
                >
                  <GroupedRow title="Agent could not be created" />
                </GroupedList>
              ) : null}
            </DialogBody>
            <GroupedDialogActions
              intent="create"
              label="new agent"
              dirty={form.formState.isDirty}
              pending={create.isPending}
              disabled={!form.formState.isValid}
              save={form.handleSubmit(submit)}
              discard={() => {
                form.reset();
                setError(undefined);
              }}
              saveLabel="Create agent"
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

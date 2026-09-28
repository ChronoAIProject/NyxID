import { useState } from "react";
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
import { useKeys } from "@/hooks/use-keys";
import { useCreateNyxBotAgent } from "@/hooks/use-nyxbot-agents";
import {
  assistantAgentCreateSchema,
  type AssistantAgentCreate,
} from "@/schemas/assistant-nyxagent";

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
}: {
  readonly value: readonly string[];
  readonly onChange: (slugs: string[]) => void;
  readonly disabled?: boolean;
}) {
  const keys = useKeys();
  const services = (keys.data ?? [])
    .filter((key) => key.is_active)
    .map((key) => ({ slug: key.slug, label: key.label }));
  const known = new Set(services.map((service) => service.slug));
  const rows = [
    ...services,
    ...value.filter((slug) => !known.has(slug)).map((slug) => ({ slug, label: slug })),
  ];
  function toggle(slug: string, checked: boolean) {
    onChange(checked ? [...value, slug] : value.filter((item) => item !== slug));
  }
  if (keys.isPending) {
    return <p className="text-[12px] text-text-tertiary">Loading your services...</p>;
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
              <span className="min-w-0 flex-1 truncate text-foreground">{service.label}</span>
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

/** Create a specialist: its name, role and starting grants. */
export function NewAgentDialog({
  onClose,
  onCreated,
}: {
  readonly onClose: () => void;
  readonly onCreated: (created: { id: string; home_conversation_id: string }) => void;
}) {
  const create = useCreateNyxBotAgent();
  const form = useAppForm<AssistantAgentCreate>({
    resolver: zodResolver(assistantAgentCreateSchema),
    defaultValues: { name: "", description: "", services: [], account_read: false },
  });
  const [error, setError] = useState<string>();
  const name = form.watch("name");
  const description = form.watch("description");

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
            A specialist keeps its own memory and threads, uses only the services you grant here,
            and asks NyxBot for anything else.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form noValidate onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-4 pb-1">
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
                      Lowercase letters, digits and hyphens.
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
                name="services"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Services it may use</FormLabel>
                    <ServiceGrantPicker value={field.value} onChange={field.onChange} />
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="account_read"
                render={({ field }) => (
                  <FormItem>
                    <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
                      <div className="space-y-1">
                        <FormLabel>Read my account</FormLabel>
                        <FormDescription className="text-[12px]">
                          Lets it look up your keys, services and nodes. It can never change or
                          delete them.
                        </FormDescription>
                      </div>
                      <FormControl>
                        <Switch checked={field.value} onCheckedChange={field.onChange} />
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

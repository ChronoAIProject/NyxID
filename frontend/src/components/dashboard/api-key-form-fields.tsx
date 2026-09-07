import type { UseFormReturn } from "react-hook-form";
import { API_KEY_SCOPES, type CreateApiKeyFormData } from "@/schemas/api-keys";
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { DatePicker } from "@/components/ui/date-picker";
import { Checkbox } from "@/components/ui/checkbox";
import { useId } from "react";

type Props = { form: UseFormReturn<CreateApiKeyFormData> };

export function ApiKeyResourceFields({
  form,
  services,
  nodes,
}: Props & {
  services: readonly { id: string; name: string }[];
  nodes: readonly { id: string; name: string }[];
}) {
  const prefix = useId();
  const allServices = form.watch("allow_all_services");
  const allNodes = form.watch("allow_all_nodes");
  return (
    <div className="space-y-4">
      {(["services", "nodes"] as const).map((kind) => {
        const allField =
          kind === "services" ? "allow_all_services" : "allow_all_nodes";
        const idsField =
          kind === "services" ? "allowed_service_ids" : "allowed_node_ids";
        const all = kind === "services" ? allServices : allNodes;
        const resources = kind === "services" ? services : nodes;
        return (
          <section
            key={kind}
            className="space-y-2 border-t border-border/50 pt-3"
          >
            <FormField
              control={form.control}
              name={allField}
              render={({ field }) => (
                <FormItem>
                  <div className="flex items-center justify-between gap-3">
                    <FormLabel>
                      {kind === "services" ? "Services" : "Nodes"}
                    </FormLabel>
                    <label className="flex items-center gap-2 text-[12px]">
                      <Checkbox
                        checked={field.value ?? false}
                        onCheckedChange={(checked) => {
                          field.onChange(checked === true);
                          if (checked === true) form.setValue(idsField, []);
                        }}
                      />
                      Allow all {kind}
                    </label>
                  </div>
                </FormItem>
              )}
            />
            {!all && (
              <FormField
                control={form.control}
                name={idsField}
                render={({ field }) => (
                  <FormItem>
                    <div className="max-h-48 space-y-2 overflow-y-auto">
                      {resources.map((resource) => (
                        <label
                          key={resource.id}
                          htmlFor={`${prefix}-${kind}-${resource.id}`}
                          className="flex items-start gap-2 text-[12px]"
                        >
                          <Checkbox
                            id={`${prefix}-${kind}-${resource.id}`}
                            checked={(field.value ?? []).includes(resource.id)}
                            onCheckedChange={() =>
                              field.onChange(
                                toggleInArray(field.value ?? [], resource.id),
                              )
                            }
                          />
                          <span className="min-w-0 break-words">
                            {resource.name}
                          </span>
                        </label>
                      ))}
                      {resources.length === 0 && (
                        <p className="text-[12px] text-muted-foreground">
                          No {kind} available.
                        </p>
                      )}
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
          </section>
        );
      })}
    </div>
  );
}

function toggleInArray(
  items: readonly string[],
  item: string,
): readonly string[] {
  return items.includes(item)
    ? items.filter((value) => value !== item)
    : [...items, item];
}

export function ApiKeyNameField({ form }: Props) {
  return (
    <FormField
      control={form.control}
      name="name"
      render={({ field }) => (
        <FormItem>
          <FormLabel>Name</FormLabel>
          <FormControl>
            <Input placeholder="My API Key" {...field} />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

export function ApiKeyScopesField({ form }: Props) {
  return (
    <FormField
      control={form.control}
      name="scopes"
      render={({ field }) => (
        <FormItem>
          <FormLabel>Scopes</FormLabel>
          <div className="flex flex-wrap gap-2">
            {API_KEY_SCOPES.map((scope) => {
              const isSelected = (field.value as readonly string[]).includes(
                scope,
              );
              return (
                <label
                  key={scope}
                  className="flex cursor-pointer items-center gap-2 text-[12px]"
                >
                  <Checkbox
                    checked={isSelected}
                    onCheckedChange={() =>
                      field.onChange(
                        toggleInArray(field.value as readonly string[], scope),
                      )
                    }
                  />
                  {scope}
                </label>
              );
            })}
          </div>
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

export function ApiKeyExpiryField({ form }: Props) {
  return (
    <FormField
      control={form.control}
      name="expires_at"
      render={({ field }) => (
        <FormItem>
          <FormLabel>
            Expiry Date{" "}
            <span className="text-muted-foreground">(optional)</span>
          </FormLabel>
          <FormControl>
            <DatePicker
              value={field.value ?? null}
              onChange={(v) => field.onChange(v)}
              minDate={new Date().toISOString().slice(0, 10)}
              placeholder="No expiry"
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

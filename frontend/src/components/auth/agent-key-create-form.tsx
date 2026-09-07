import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  createApiKeySchema,
  type CreateApiKeyFormData,
} from "@/schemas/api-keys";
import type { AgentKeyOptions } from "@/schemas/agent-key-login";
import { PLATFORM_OPTIONS } from "@/schemas/agent-bindings";
import {
  ApiKeyNameField,
  ApiKeyScopesField,
  ApiKeyExpiryField,
  ApiKeyResourceFields,
} from "@/components/dashboard/api-key-form-fields";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ArrowRight } from "lucide-react";

export function AgentKeyCreateForm({
  options,
  label,
  initialValues,
  disabled,
  onReview,
}: {
  options: AgentKeyOptions;
  label: string;
  initialValues?: CreateApiKeyFormData;
  disabled: boolean;
  onReview: (data: CreateApiKeyFormData) => void;
}) {
  const [expiryChoice, setExpiryChoice] = useState(
    initialValues ? (initialValues.expires_at ? "custom" : "none") : "90",
  );
  const [defaultExpiry] = useState(() =>
    new Date(Date.now() + 90 * 86400000).toISOString(),
  );
  const form = useAppForm<CreateApiKeyFormData>({
    resolver: zodResolver(createApiKeySchema),
    defaultValues: initialValues ?? {
      name: label,
      scopes: ["read", "proxy"],
      allow_all_services: false,
      allow_all_nodes: false,
      allowed_service_ids: [],
      allowed_node_ids: [],
      expires_at: defaultExpiry,
      platform: "generic",
    },
  });
  const owner = form.watch("target_org_id");
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onReview)}>
        <fieldset disabled={disabled} className="space-y-4">
          <ApiKeyNameField form={form} />
          {options.orgs.length > 0 && (
            <FormField
              control={form.control}
              name="target_org_id"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Owner</FormLabel>
                  <Select
                    value={field.value ?? "personal"}
                    onValueChange={(value) => {
                      field.onChange(value === "personal" ? undefined : value);
                      form.setValue("allowed_service_ids", []);
                      form.setValue("allowed_node_ids", []);
                      form.setValue("allow_all_services", false);
                      form.setValue("allow_all_nodes", false);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="personal">Personal</SelectItem>
                      {options.orgs.map((org) => (
                        <SelectItem key={org.id} value={org.id}>
                          {org.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
          )}
          <ApiKeyScopesField form={form} />
          <ApiKeyResourceFields
            form={form}
            services={options.services.filter(
              (item) => !owner || item.owner_id === owner,
            )}
            nodes={options.nodes.filter(
              (item) => !owner || item.owner_id === owner,
            )}
          />
          <div className="space-y-2">
            <label
              className="text-[12px] font-medium"
              htmlFor="agent-key-expiry-choice"
            >
              Key expiry
            </label>
            <Select
              value={expiryChoice}
              onValueChange={(value) => {
                setExpiryChoice(value);
                if (value !== "custom")
                  form.setValue(
                    "expires_at",
                    value === "none"
                      ? null
                      : new Date(
                          Date.now() + Number(value) * 86400000,
                        ).toISOString(),
                  );
              }}
            >
              <SelectTrigger id="agent-key-expiry-choice">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {["7", "30", "90", "365"].map((days) => (
                  <SelectItem key={days} value={days}>
                    {days} days
                  </SelectItem>
                ))}
                <SelectItem value="custom">Custom date</SelectItem>
                <SelectItem value="none">No expiry</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {expiryChoice === "custom" && <ApiKeyExpiryField form={form} />}
          <div className="grid gap-3 sm:grid-cols-2">
            {(["rate_limit_per_second", "rate_limit_burst"] as const).map(
              (name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {name === "rate_limit_per_second"
                          ? "Requests per second"
                          : "Burst"}
                      </FormLabel>
                      <Input
                        type="number"
                        min={1}
                        step={1}
                        placeholder="Default"
                        value={field.value ?? ""}
                        onChange={(event) =>
                          field.onChange(
                            event.target.value === ""
                              ? undefined
                              : Number(event.target.value),
                          )
                        }
                      />
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ),
            )}
          </div>
          <FormField
            control={form.control}
            name="platform"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Platform</FormLabel>
                <Select
                  value={field.value ?? "generic"}
                  onValueChange={field.onChange}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PLATFORM_OPTIONS.map((platform) => (
                      <SelectItem key={platform} value={platform}>
                        {platform}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
          <div className="flex justify-end">
            <Button
              type="submit"
              variant="primary"
              disabled={
                disabled ||
                !form.watch("name").trim() ||
                form.watch("scopes").length === 0
              }
            >
              <ArrowRight className="size-3" />
              Review permissions
            </Button>
          </div>
        </fieldset>
      </form>
    </Form>
  );
}

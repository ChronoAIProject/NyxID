import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PlatformServiceScope } from "@/components/shared/platform-service-scope";
import { Checkbox } from "@/components/ui/checkbox";
import {
  ApiKeyNameField,
  ApiKeyScopesField,
  ApiKeyExpiryField,
} from "@/components/dashboard/api-key-form-fields";
import {
  createApiKeySchema,
  type CreateApiKeyFormData,
} from "@/schemas/api-keys";
import type { RequestedPermissions } from "@/schemas/login-request";
import {
  compareKey,
  draftSummary,
  permissionOptions,
  type LoginInventory,
} from "@/lib/login-permissions";
import { LoginPermissionPicker } from "./login-permission-picker";
import { LoginAccessRows } from "./login-access-rows";
import { LoginGrantReview } from "./login-grant-review";

export function LoginKeyDraft({
  initial,
  inventory,
  requested,
  actor,
  disabled,
  onDraft,
  onApprove,
  initialRequested,
  onRequestedChange,
  onDeny,
}: {
  onDeny: () => void;
  initial: CreateApiKeyFormData;
  inventory: LoginInventory;
  requested: RequestedPermissions;
  actor: string;
  disabled: boolean;
  onDraft: (data: CreateApiKeyFormData) => void;
  onApprove: (data: CreateApiKeyFormData) => void;
  initialRequested: RequestedPermissions;
  onRequestedChange: (value: RequestedPermissions) => void;
}) {
  const form = useAppForm<CreateApiKeyFormData>({
    resolver: zodResolver(createApiKeySchema),
    defaultValues: initial,
  });
  useEffect(() => {
    const subscription = form.watch(() => onDraft(form.getValues()));
    return () => subscription.unsubscribe();
  }, [form, onDraft]);
  const data = form.watch();
  const summary = draftSummary(data, inventory, actor);
  const comparison = compareKey(summary, requested, inventory);
  const selected = data.allowed_service_ids ?? [];
  const implied = summary.allowed_services
    .filter((s) => !selected.includes(s.id))
    .map((s) => s.id);
  const ownerServices = inventory.options.services.filter(
    (s) => !data.target_org_id || s.owner_id === data.target_org_id,
  );
  const toggleService = (id: string) =>
    form.setValue(
      "allowed_service_ids",
      selected.includes(id)
        ? selected.filter((v) => v !== id)
        : [...selected, id],
    );
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onApprove)} className="space-y-4">
        <fieldset disabled={disabled} className="space-y-4">
          <LoginGrantReview
            apiKey={summary}
            comparison={comparison}
            kind="new"
            customize={
              <>
                <h4 className="text-[12px] font-semibold">
                  Requested access filters
                </h4>
                <LoginPermissionPicker
                  options={permissionOptions(inventory)}
                  value={requested}
                  initial={initialRequested}
                  disabled={disabled}
                  onChange={onRequestedChange}
                  connections={{
                    inventory,
                    organizationId: data.target_org_id,
                    selectedIds: selected,
                    impliedIds: implied,
                    allowAll: data.allow_all_services ?? false,
                    onToggle: toggleService,
                  }}
                />
                <ApiKeyNameField form={form} />
                {inventory.options.orgs.length > 0 && (
                  <label className="block space-y-2">
                    Owner
                    <Select
                      disabled={disabled}
                      value={data.target_org_id ?? "personal"}
                      onValueChange={(value) => {
                        form.setValue(
                          "target_org_id",
                          value === "personal" ? undefined : value,
                        );
                        form.setValue("allowed_service_ids", []);
                        form.setValue("allowed_node_ids", []);
                        form.setValue("allow_all_services", false);
                        form.setValue("allow_auto_connected_services", false);
                        form.setValue("allow_all_nodes", false);
                      }}
                    >
                      <SelectTrigger aria-label="Owner">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="personal">Personal</SelectItem>
                        {inventory.options.orgs.map((org) => (
                          <SelectItem key={org.id} value={org.id}>
                            {org.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                )}
                <PlatformServiceScope
                  services={ownerServices.map((s) => ({
                    ...s,
                    platform_grant_eligible: s.owner_id === summary.owner_id,
                  }))}
                  selectedIds={selected}
                  allowAll={data.allow_auto_connected_services}
                  onAllowAllChange={(value) =>
                    form.setValue("allow_auto_connected_services", value)
                  }
                  onToggle={toggleService}
                  orgOwned={!!data.target_org_id}
                  disabled={disabled || data.allow_all_services}
                />
                <ApiKeyExpiryField form={form} />
                <p className="text-[11px] text-muted-foreground">
                  Dates expire at the end of the selected day (23:59:59 UTC).
                  {summary.expires_at && (
                    <span className="block">
                      {new Date(summary.expires_at).toLocaleString()}
                    </span>
                  )}
                </p>
                <label className="block space-y-2">
                  Platform
                  <Select
                    disabled={disabled}
                    value={data.platform ?? "generic"}
                    onValueChange={(value) => form.setValue("platform", value)}
                  >
                    <SelectTrigger aria-label="Platform">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {["generic", "codex", "claude-code", "openclaw"].map(
                        (p) => (
                          <SelectItem key={p} value={p}>
                            {p}
                          </SelectItem>
                        ),
                      )}
                    </SelectContent>
                  </Select>
                </label>
                <label className="flex items-center gap-2">
                  <Checkbox
                    checked={data.allow_all_services}
                    onCheckedChange={(v) => {
                      form.setValue("allow_all_services", v === true);
                      form.setValue("allowed_service_ids", []);
                    }}
                  />
                  All current and future services
                </label>
                <label className="flex items-center gap-2">
                  <Checkbox
                    checked={data.allow_all_nodes}
                    onCheckedChange={(v) => {
                      form.setValue("allow_all_nodes", v === true);
                      form.setValue("allowed_node_ids", []);
                    }}
                  />
                  All current and future nodes
                </label>
                {inventory.options.nodes
                  .filter(
                    (n) =>
                      !data.target_org_id || n.owner_id === data.target_org_id,
                  )
                  .map((node) => (
                    <label key={node.id} className="flex items-center gap-2">
                      <Checkbox
                        aria-label={`Grant node ${node.name}`}
                        disabled={data.allow_all_nodes}
                        checked={data.allowed_node_ids?.includes(node.id)}
                        onCheckedChange={(v) =>
                          form.setValue(
                            "allowed_node_ids",
                            v
                              ? [...(data.allowed_node_ids ?? []), node.id]
                              : data.allowed_node_ids?.filter(
                                  (id) => id !== node.id,
                                ),
                          )
                        }
                      />
                      {node.name}
                    </label>
                  ))}
                {(["rate_limit_per_second", "rate_limit_burst"] as const).map(
                  (name) => (
                    <label key={name} className="block">
                      {name === "rate_limit_burst"
                        ? "Burst"
                        : "Requests per second"}
                      <Input
                        aria-label={
                          name === "rate_limit_burst"
                            ? "Burst"
                            : "Requests per second"
                        }
                        className="mt-2 block h-8 w-full rounded-lg border border-input bg-background px-3"
                        type="number"
                        min="1"
                        max="4294967295"
                        placeholder="Default"
                        value={data[name] ?? ""}
                        onChange={(e) =>
                          form.setValue(
                            name,
                            e.target.value === ""
                              ? undefined
                              : Number(e.target.value),
                          )
                        }
                      />
                      {form.formState.errors[name] && (
                        <span className="text-destructive">
                          {form.formState.errors[name]?.message}
                        </span>
                      )}
                    </label>
                  ),
                )}
              </>
            }
            actions={
              <div className="space-y-3">
                {!comparison.matches && (
                  <p role="alert" className="text-[12px] text-warning">
                    Complete the highlighted access selections, or edit the
                    filters in Customize.
                  </p>
                )}
                <div className="flex flex-wrap justify-end gap-2">
                  <Button
                    type="button"
                    variant="destructive"
                    onClick={onDeny}
                    disabled={disabled}
                  >
                    Deny request
                  </Button>
                  <Button
                    type="submit"
                    variant="primary"
                    disabled={
                      disabled ||
                      !data.name.trim() ||
                      !comparison.matches ||
                      !createApiKeySchema.safeParse(data).success
                    }
                    isLoading={disabled}
                  >
                    Create &amp; continue
                  </Button>
                </div>
              </div>
            }
          >
            <LoginAccessRows
              apiKey={summary}
              comparison={comparison}
              requested={requested}
              inventory={inventory}
              disabled={disabled}
              scopes={<ApiKeyScopesField form={form} />}
              selection={{
                inventory,
                organizationId: data.target_org_id,
                selectedIds: selected,
                impliedIds: implied,
                allowAll: data.allow_all_services ?? false,
                onToggle: toggleService,
              }}
            />
          </LoginGrantReview>
        </fieldset>
      </form>
    </Form>
  );
}

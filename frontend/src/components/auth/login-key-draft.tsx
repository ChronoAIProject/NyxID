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
  connectionReady,
  connectionSlug,
  connectionCovers,
  requestedGroups,
  permissionOptions,
  canonicalScope,
  type LoginInventory,
  type AccessEntry,
} from "@/lib/login-permissions";
import { PermissionIcon } from "./login-permission-picker";
import {
  AgentKeyPermissions,
  AgentKeyIssuanceNotice,
} from "./agent-key-permissions";

export function AccessEntries({
  label,
  entries,
}: {
  label: string;
  entries: AccessEntry[];
}) {
  const groups = new Map<string, AccessEntry[]>();
  for (const entry of entries)
    groups.set(`${entry.group}:${entry.service}`, [
      ...(groups.get(`${entry.group}:${entry.service}`) ?? []),
      entry,
    ]);
  if (!entries.length) return null;
  return (
    <div className="space-y-2 text-[11px]">
      <h4 className="font-semibold">{label}</h4>
      {[...groups].map(([id, items]) => (
        <div key={id} className="space-y-1">
          <div className="flex items-center gap-2 text-muted-foreground">
            <PermissionIcon group={items[0]!.group} />
            {items[0]!.service}
          </div>
          <ul className="flex flex-wrap gap-1 pl-6">
            {items.map((entry, index) => (
              <li
                key={`${entry.label}:${index}`}
                title={
                  [entry.scope, entry.description]
                    .filter(Boolean)
                    .join(" — ") || undefined
                }
                className="max-w-full break-all rounded-md border border-border px-2 py-1"
              >
                {entry.label}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

export function LoginKeyDraft({
  initial,
  inventory,
  requested,
  actor,
  disabled,
  onDraft,
  onApprove,
}: {
  initial: CreateApiKeyFormData;
  inventory: LoginInventory;
  requested: RequestedPermissions;
  actor: string;
  disabled: boolean;
  onDraft: (data: CreateApiKeyFormData) => void;
  onApprove: (data: CreateApiKeyFormData) => void;
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
  const groups = requestedGroups(requested);
  const displayOptions = permissionOptions(inventory);
  const services = ownerServices
    .map((service) => {
      const connection = inventory.connections.find((c) => c.id === service.id);
      const scopes = connection
        ? groups.get(connectionSlug(connection))
        : undefined;
      const relevant =
        scopes !== undefined ||
        requested.service_permissions.some((p) => !p.includes("::"));
      const covers =
        !!connection && connectionCovers(connection, scopes ?? [], requested);
      const required = [
        ...(scopes ?? []),
        ...requested.service_permissions.filter((p) => !p.includes("::")),
      ];
      const extras =
        connection?.granted_scopes?.filter(
          (p) =>
            !required.includes(canonicalScope(connectionSlug(connection), p)),
        ).length ?? Infinity;
      return { service, relevant, covers, extras };
    })
    .filter(
      ({ service, relevant, covers }) =>
        selected.includes(service.id) ||
        implied.includes(service.id) ||
        (relevant && covers),
    )
    .sort(
      (a, b) =>
        a.extras - b.extras || a.service.name.localeCompare(b.service.name),
    )
    .map(({ service }) => service);
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onApprove)} className="space-y-4">
        <fieldset disabled={disabled} className="space-y-4">
          <ApiKeyNameField form={form} />
          <div className="space-y-2">
            <h3 className="text-[12px] font-semibold">
              Service connections · {summary.allowed_services.length} granted
            </h3>
            <p className="text-[11px] text-muted-foreground">
              These connections determine the actual provider access. Requested
              filters cannot remove their permissions.
            </p>
            <div className="max-h-56 space-y-2 overflow-y-auto rounded-xl border border-border p-3">
              {services.map((service) => {
                const connection = inventory.connections.find(
                  (c) => c.id === service.id,
                );
                const ready = connection && connectionReady(connection);
                return (
                  <label
                    key={service.id}
                    className="flex items-start gap-2 rounded-lg border border-border/50 p-2 text-[12px]"
                  >
                    <Checkbox
                      aria-label={`Grant connection ${service.name}`}
                      disabled={
                        !ready ||
                        data.allow_all_services ||
                        implied.includes(service.id)
                      }
                      checked={
                        selected.includes(service.id) ||
                        implied.includes(service.id)
                      }
                      onCheckedChange={(v) =>
                        form.setValue(
                          "allowed_service_ids",
                          v
                            ? [...selected, service.id]
                            : selected.filter((id) => id !== service.id),
                        )
                      }
                    />
                    <div className="min-w-0 space-y-1">
                      <span className="flex items-center gap-2">
                        <PermissionIcon
                          group={
                            connection ? connectionSlug(connection) : "unknown"
                          }
                        />
                        {service.name}
                      </span>
                      <div className="flex flex-wrap gap-1 text-[11px] text-muted-foreground">
                        {connection?.granted_scopes?.length
                          ? connection.granted_scopes.map((scope) => {
                              const option = displayOptions.find(
                                (o) =>
                                  o.value ===
                                  `${connectionSlug(connection)}::${canonicalScope(connectionSlug(connection), scope)}`,
                              );
                              return (
                                <span
                                  key={scope}
                                  title={[scope, option?.description]
                                    .filter(Boolean)
                                    .join(" — ")}
                                  className="rounded border border-border px-1.5 py-0.5"
                                >
                                  {option?.label ?? scope}
                                </span>
                              );
                            })
                          : "Provider access not reported"}
                      </div>
                      {connection?.node_id && (
                        <p className="text-[11px] text-warning">
                          Requires node access:{" "}
                          {inventory.options.nodes.find(
                            (n) => n.id === connection.node_id,
                          )?.name ?? connection.node_id}
                        </p>
                      )}
                      {!ready && (
                        <p className="text-[11px] text-warning">
                          Connection unavailable
                        </p>
                      )}
                    </div>
                  </label>
                );
              })}
              {!services.length && (
                <p className="text-[12px] text-muted-foreground">
                  No matching connections. Edit the requested permissions or
                  connect a service from AI Services.
                </p>
              )}
            </div>
          </div>
          <details className="rounded-xl border border-border p-3 text-[12px]">
            <summary className="cursor-pointer font-medium">
              Key settings and actual NyxID grant
            </summary>
            <div className="mt-3 space-y-4">
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
              <ApiKeyScopesField form={form} />
              <ApiKeyExpiryField form={form} />
              <p className="text-[11px] text-muted-foreground">
                Dates expire at the end of the selected day (23:59:59 UTC).
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
            </div>
          </details>
          <div className="space-y-3 rounded-xl border border-border p-3">
            <h3 className="text-[12px] font-semibold">Actual grant</h3>
            <AgentKeyPermissions apiKey={summary} />
            <AccessEntries
              label="Also grants — included with this key"
              entries={comparison.extras}
            />
          </div>
          {!comparison.matches && (
            <p role="alert" className="text-[12px] text-warning">
              {comparison.missing.join(". ")}. Choose connections/permissions or
              edit the requested filters.
            </p>
          )}
          <AgentKeyIssuanceNotice existing={false} />
          <p className="text-[12px] text-muted-foreground">
            Create &amp; continue creates this key and approves the requesting
            device with the displayed access.
          </p>
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
        </fieldset>
      </form>
    </Form>
  );
}

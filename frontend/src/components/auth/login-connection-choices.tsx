import { Link2 } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import {
  canonicalScope,
  connectionCovers,
  connectionReady,
  connectionSlug,
  permissionOptions,
  requestedGroups,
  type LoginInventory,
} from "@/lib/login-permissions";
import type { RequestedPermissions } from "@/schemas/login-request";

export type ConnectionSelection = {
  inventory: LoginInventory;
  organizationId?: string;
  selectedIds: string[];
  impliedIds: string[];
  allowAll: boolean;
  onToggle: (id: string) => void;
};

function loginConnectionChoices(
  selection: ConnectionSelection,
  requested: RequestedPermissions,
) {
  const groups = requestedGroups(requested);
  const options = permissionOptions(selection.inventory);
  return selection.inventory.options.services
    .filter(
      (s) =>
        !selection.organizationId || s.owner_id === selection.organizationId,
    )
    .flatMap((service) => {
      const connection = selection.inventory.connections.find(
        (c) => c.id === service.id,
      );
      if (!connection) return [];
      const group = connectionSlug(connection);
      const required = [
        ...(groups.get(group) ?? []),
        ...requested.service_permissions.filter((p) => !p.includes("::")),
      ];
      const constrained = groups.has(group) || required.length > 0;
      const ready = connectionReady(connection);
      const covers = connectionCovers(connection, required, requested);
      const permissions = (connection.granted_scopes ?? []).map((scope) => {
        const option = options.find(
          (o) => o.value === `${group}::${canonicalScope(group, scope)}`,
        );
        return {
          scope,
          label: option?.label ?? scope,
          description: option?.description,
          requested: required.includes(canonicalScope(group, scope)),
        };
      });
      const extras = permissions.filter(
        (p) => !required.includes(canonicalScope(group, p.scope)),
      ).length;
      const status = !ready
        ? "Unavailable"
        : !connection.granted_scopes?.length
          ? "Provider access not reported"
          : constrained && covers
            ? extras
              ? `Matched + ${extras} extra${extras === 1 ? "" : "s"}`
              : "Exact match"
            : constrained
              ? "Missing requested permissions"
              : `${permissions.length} permission${permissions.length === 1 ? "" : "s"}`;
      return [
        {
          service,
          connection,
          group,
          permissions,
          extras,
          covers,
          constrained,
          ready,
          status,
        },
      ];
    })
    .sort(
      (a, b) =>
        Number(b.ready) - Number(a.ready) ||
        Number(b.covers) - Number(a.covers) ||
        a.extras - b.extras ||
        a.service.name.localeCompare(b.service.name),
    );
}

export function LoginConnectionChoices({
  selection,
  requested,
  group,
  disabled,
}: {
  selection: ConnectionSelection;
  requested: RequestedPermissions;
  group: string;
  disabled: boolean;
}) {
  const choices = loginConnectionChoices(selection, requested).filter(
    (c) => c.group === group,
  );
  return (
    <div className="mt-2 space-y-1 rounded-lg border border-border/70 bg-background/50 p-2">
      <p className="mb-2 flex items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
        <Link2 className="size-3" />
        {choices.length ? "Use an existing connection" : "Not connected"}
      </p>
      {choices.map(({ service, connection, permissions, ready, status }) => {
        const implied =
          selection.allowAll || selection.impliedIds.includes(service.id);
        const checked = implied || selection.selectedIds.includes(service.id);
        return (
          <label
            key={service.id}
            className={`flex cursor-pointer items-start gap-2 rounded-md border p-2 ${checked ? "border-primary/30 bg-primary/5" : "border-transparent hover:bg-muted/40"}`}
          >
            <Checkbox
              aria-label={`Grant connection ${service.name}`}
              disabled={disabled || implied || (!ready && !checked)}
              checked={checked}
              onCheckedChange={() => selection.onToggle(service.id)}
              className="mt-0.5"
            />
            <span className="min-w-0 flex-1 space-y-1">
              <span className="flex flex-wrap items-center justify-between gap-1">
                <span className="font-medium">
                  {connection.label || service.name}
                </span>
                <span
                  className={`text-[10px] ${status === "Exact match" ? "text-success" : "text-muted-foreground"}`}
                >
                  {status}
                </span>
              </span>
              <span className="block break-words text-[10px] text-muted-foreground">
                {service.name}
                {implied ? " · Included by your broader grant" : ""}
              </span>
              <span className="flex flex-wrap gap-1">
                {permissions.slice(0, 3).map((p) => (
                  <span
                    key={p.scope}
                    title={[p.scope, p.description].filter(Boolean).join(" · ")}
                    className="max-w-full break-words rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground"
                  >
                    {p.label}
                  </span>
                ))}
                {permissions.length > 3 && (
                  <span className="text-[10px] text-muted-foreground">
                    +{permissions.length - 3} more
                  </span>
                )}
              </span>
              {connection.node_id && (
                <span className="block text-[10px] text-warning">
                  Requires node access:{" "}
                  {selection.inventory.options.nodes.find(
                    (n) => n.id === connection.node_id,
                  )?.name ?? connection.node_id}
                </span>
              )}
            </span>
          </label>
        );
      })}
      {!choices.length && (
        <p className="text-[11px] text-muted-foreground">
          Connect an account in AI Services to grant this access. You can still
          use its permissions as filters.
        </p>
      )}
    </div>
  );
}

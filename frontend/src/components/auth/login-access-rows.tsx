import type { ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import type { AgentKeySummary } from "@/schemas/agent-key-login";
import type { RequestedPermissions } from "@/schemas/login-request";
import {
  connectionCovers,
  connectionSlug,
  effectiveLoginConnections,
  permissionOptions,
  requestedGroups,
  type LoginInventory,
  type PermissionComparison,
} from "@/lib/login-permissions";
import { PermissionIcon } from "./login-permission-picker";
import { AccessEntries } from "./login-grant-review";
import {
  LoginConnectionChoices,
  type ConnectionSelection,
} from "./login-connection-choices";

export function LoginAccessRows({
  apiKey,
  comparison,
  requested,
  inventory,
  selection,
  scopes,
  disabled,
}: {
  apiKey: AgentKeySummary;
  comparison: PermissionComparison;
  requested: RequestedPermissions;
  inventory: LoginInventory;
  selection?: ConnectionSelection;
  scopes?: ReactNode;
  disabled: boolean;
}) {
  const required = requestedGroups(requested);
  const connections = effectiveLoginConnections(apiKey, inventory);
  const options = permissionOptions(inventory);
  const groups = [
    ...new Set([
      "nyxid",
      ...required.keys(),
      ...comparison.matched.map((e) => e.group),
      ...comparison.extras.map((e) => e.group),
    ]),
  ];
  return (
    <div className="divide-y divide-border rounded-xl border border-border">
      {groups.map((group) => {
        const matched = comparison.matched.filter((e) => e.group === group);
        const extras = comparison.extras.filter((e) => e.group === group);
        const accounts = connections.filter((c) => connectionSlug(c) === group);
        const name =
          group === "nyxid"
            ? "NyxID"
            : (options.find((o) => o.group === group)?.service ??
              matched[0]?.service ??
              extras[0]?.service ??
              group);
        const missing =
          required.has(group) &&
          !accounts.some(
            (c) =>
              connectionCovers(c, required.get(group) ?? [], requested) &&
              (!c.node_id ||
                apiKey.allow_all_nodes ||
                apiKey.allowed_node_ids.includes(c.node_id)),
          );
        const nyxidMissing =
          group === "nyxid" &&
          comparison.missing.some((m) => m.startsWith("NyxID"));
        const requestedEntries = options
          .filter(
            (o) => o.group === group && requested[o.field].includes(o.value),
          )
          .map((o) => ({
            group,
            service: name,
            label: o.label,
            scope: o.value,
            description: o.description,
          }));
        return (
          <details
            key={group}
            className="group/row"
            aria-label={`${name} access`}
          >
            <summary className="flex cursor-pointer list-none items-center gap-3 p-3 focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
              <PermissionIcon group={group} />
              <span className="min-w-0 flex-1">
                <strong className="block break-words text-[12px]">
                  {name}
                </strong>
                <span className="block break-words text-[11px] text-muted-foreground">
                  {group === "nyxid"
                    ? apiKey.scopes
                    : accounts.length
                      ? accounts.map((c) => c.label).join(", ")
                      : missing
                        ? `${requestedEntries.length || 1} requested permission${requestedEntries.length === 1 ? "" : "s"}`
                        : `${matched.length + extras.length} permissions`}
                </span>
              </span>
              <span
                className={`shrink-0 text-[10px] ${missing || nyxidMissing || extras.length ? "text-warning" : "text-muted-foreground"}`}
              >
                {missing
                  ? accounts.length
                    ? "Check access"
                    : "Choose account"
                  : nyxidMissing
                    ? "Choose permissions"
                    : extras.length
                      ? `${matched.length ? "Matched + " : "+ "}${extras.length} extra${extras.length === 1 ? "" : "s"}`
                      : `${matched.length} selected`}
              </span>
              <ChevronDown
                aria-hidden="true"
                className="size-3 shrink-0 text-muted-foreground transition-transform group-open/row:rotate-180"
              />
            </summary>
            <div className="space-y-3 border-t border-border/50 p-3">
              {selection && requestedEntries.length > 0 && (
                <AccessEntries label="Requested" entries={requestedEntries} />
              )}
              {group === "nyxid" && scopes}
              {selection &&
                group !== "nyxid" &&
                group !== "nodes" &&
                !group.startsWith("missing:") && (
                  <LoginConnectionChoices
                    selection={selection}
                    requested={requested}
                    group={group}
                    disabled={disabled}
                  />
                )}
              <AccessEntries label="Included permissions" entries={matched} />
              <AccessEntries label="Extra access included" entries={extras} />
              {selection && group !== "nyxid" && accounts.length > 0 && (
                <p className="text-[11px] text-muted-foreground">
                  The selected accounts keep all their provider permissions.
                </p>
              )}
            </div>
          </details>
        );
      })}
    </div>
  );
}

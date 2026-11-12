import { useUiTranslation } from "@/i18n/use-ui-translation";
import type { ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import type { AgentKeySummary } from "@/schemas/agent-key-login";
import type { RequestedPermissions } from "@/schemas/login-request";
import {
  connectionCovers,
  connectionSlug,
  connectionOrganization,
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
  const { t } = useUiTranslation();
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
            open
            className="group/row"
            aria-label={t("{{name}} access", { name: name })}
          >
            <summary className="flex cursor-pointer list-none items-center gap-3 p-3 focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
              <PermissionIcon group={group} />
              <span className="min-w-0 flex-1">
                <strong className="block break-words text-12">
                  {name}
                </strong>
                <span className="block break-words text-11 text-muted-foreground">
                  {group === "nyxid"
                    ? apiKey.scopes
                    : accounts.length
                      ? accounts.map((c) => {
                          const organization = connectionOrganization(c, inventory);
                          const owner = organization
                            ? t("Organization · {{org_name}}", { org_name: organization })
                            : t("Personal");
                          return `${c.label} (${owner})`;
                        }).join(", ")
                      : missing
                        ? t("{{count}} requested permissions", { count: requestedEntries.length || 1 })
                        : t("{{value}} permissions", { value: matched.length + extras.length })}
                </span>
              </span>
              <span
                className={`shrink-0 text-10 ${missing || nyxidMissing || extras.length ? "text-warning" : "text-muted-foreground"}`}
              >
                {missing
                  ? accounts.length
                    ? t("Check access")
                    : t("Choose account")
                  : nyxidMissing
                    ? t("Choose permissions")
                    : extras.length
                      ? t(matched.length ? "Matched + {{count}} extra permissions" : "+ {{count}} extra permissions", { count: extras.length })
                      : t("{{length}} selected", { length: matched.length })}
              </span>
              <ChevronDown
                aria-hidden="true"
                className="size-3 shrink-0 text-muted-foreground transition-transform group-open/row:rotate-180"
              />
            </summary>
            <div className="space-y-3 border-t border-border/50 p-3">
              {selection && requestedEntries.length > 0 && (
                <AccessEntries label={t("Requested")} entries={requestedEntries} />
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
              <AccessEntries label={t("Included permissions")} entries={matched} />
              <AccessEntries label={t("Extra access included")} entries={extras} />
              {selection && group !== "nyxid" && accounts.length > 0 && (
                <p className="text-11 text-muted-foreground">{t("The selected accounts keep all their provider permissions.")}</p>
              )}
            </div>
          </details>
        );
      })}
    </div>
  );
}

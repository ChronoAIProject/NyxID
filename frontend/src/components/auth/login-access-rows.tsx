import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Fragment, useState, type ReactNode } from "react";
import { ChevronDown, ChevronsDownUp, ChevronsUpDown } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AgentKeySummary } from "@/schemas/agent-key-login";
import type { RequestedPermissions } from "@/schemas/login-request";
import {
  connectionCovers,
  connectionSlug,
  connectionOwner,
  connectionOwnerLabel,
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

/** Above this many service rows, only rows that still need a choice open. */
export const AUTO_EXPAND_ROW_LIMIT = 5;

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
  // Rows the approver opened or closed (individually or via Collapse/Expand
  // all) keep that choice; others follow their default.
  const [openOverrides, setOpenOverrides] = useState<Record<string, boolean>>({});
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
  const rows = groups.map((group) => {
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
        return {
          group,
          matched,
          extras,
          accounts,
          name,
          missing,
          nyxidMissing,
          requestedEntries,
          // Short lists open fully; long lists open only rows awaiting a choice.
          defaultOpen:
            groups.length <= AUTO_EXPAND_ROW_LIMIT || missing || nyxidMissing,
        };
  });
  const isOpen = (row: (typeof rows)[number]) =>
    openOverrides[row.group] ?? row.defaultOpen;
  const anyOpen = rows.some(isOpen);
  const setAll = (open: boolean) =>
    setOpenOverrides(Object.fromEntries(groups.map((g) => [g, open])));
  return (
    <div className="space-y-2">
      <div className="sticky top-0 z-10 -mx-1 flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-card px-1 py-2 text-11">
        <p className="font-medium">{t("Allow this device to access")}</p>
        <span className="flex items-center gap-2">
          <span
            className={
              comparison.extras.length ? "text-warning" : "text-muted-foreground"
            }
          >
            {comparison.matches
              ? comparison.exact
                ? t("Exact match")
                : t("Matched + {{count}} extra permissions", { count: comparison.extras.length })
              : t("Selection needed")}
          </span>
          {rows.length > 1 && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-expanded={anyOpen}
              onClick={() => setAll(!anyOpen)}
            >
              {anyOpen ? (
                <ChevronsDownUp aria-hidden="true" />
              ) : (
                <ChevronsUpDown aria-hidden="true" />
              )}
              {anyOpen ? t("Collapse all") : t("Expand all")}
            </Button>
          )}
        </span>
      </div>
      <div className="divide-y divide-border rounded-xl border border-border">
      {rows.map((row) => {
        const { group, matched, extras, accounts, name, missing, nyxidMissing, requestedEntries } = row;
        const open = isOpen(row);
        return (
          <details
            key={group}
            open={open}
            onToggle={(e) => {
              const next = e.currentTarget.open;
              if (next !== open) {
                setOpenOverrides((prev) => ({ ...prev, [group]: next }));
              }
            }}
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
                      ? accounts.map((c, index) => {
                          const owner = connectionOwner(c, inventory, apiKey);
                          const unavailable =
                            owner.kind === "organization" && owner.unavailable;
                          return (
                            <Fragment key={c.id}>
                              {index > 0 && ", "}
                              {c.label} ({connectionOwnerLabel(owner, t)})
                              {unavailable && (
                                <>
                                  {" "}
                                  <Badge variant="secondary" className="align-middle">
                                    {t("No longer available")}
                                  </Badge>
                                </>
                              )}
                            </Fragment>
                          );
                        })
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
    </div>
  );
}

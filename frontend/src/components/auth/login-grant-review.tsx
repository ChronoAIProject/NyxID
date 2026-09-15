import type { ReactNode } from "react";
import { ChevronDown, KeyRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  effectivePermissions,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";
import type {
  AccessEntry,
  PermissionComparison,
} from "@/lib/login-permissions";
import { PermissionIcon } from "./login-permission-picker";
import {
  AgentKeyIssuanceNotice,
  AgentKeyPermissions,
} from "./agent-key-permissions";

export function AccessEntries({
  label,
  entries,
}: {
  label: string;
  entries: AccessEntry[];
}) {
  if (!entries.length) return null;
  const groups = new Map<string, AccessEntry[]>();
  for (const entry of entries) {
    const id = `${entry.group}:${entry.service}`;
    groups.set(id, [...(groups.get(id) ?? []), entry]);
  }
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

export function LoginGrantReview({
  apiKey,
  comparison,
  kind,
  children,
  credentialExpiresAt,
}: {
  apiKey: AgentKeySummary;
  comparison: PermissionComparison;
  kind: "existing" | "new";
  children?: ReactNode;
  credentialExpiresAt?: string;
}) {
  const expiry = credentialExpiresAt || apiKey.expires_at;
  return (
    <section
      aria-label="Final access review"
      className="space-y-3 rounded-xl border border-border bg-card/40 p-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <KeyRound className="size-4 text-muted-foreground" aria-hidden="true" />
        <h3 className="text-[12px] font-semibold">Final access review</h3>
        <Badge variant={kind === "new" ? "info" : "secondary"}>
          {kind === "new" ? "Creates new key" : "Existing key"}
        </Badge>
      </div>
      <p className="text-[11px] text-muted-foreground">
        {kind === "new"
          ? "This draft will create one Agent Key when you approve."
          : "This approval uses the selected key; its existing secret is unchanged."}
      </p>
      <div className="rounded-lg bg-muted/20 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <strong className="min-w-0 break-words text-[12px]">
            {apiKey.name}
          </strong>
          <span className="min-w-0 break-words text-[11px] text-muted-foreground">
            {apiKey.owner_name}
          </span>
        </div>
        <dl className="mt-3 space-y-1.5 text-[11px] [&>div]:grid [&>div]:grid-cols-[5rem_minmax(0,1fr)] [&>div]:gap-x-3">
          <div>
            <dt className="text-muted-foreground">Permissions</dt>
            <dd className="font-medium">
              {effectivePermissions(apiKey.scopes)}
            </dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Services</dt>
            <dd className="font-medium">
              {apiKey.allow_all_services
                ? "All services, including future additions"
                : `${apiKey.allowed_services.length} ${apiKey.allowed_services.length === 1 ? "service" : "services"}`}
              {!apiKey.allow_all_services &&
                apiKey.allow_auto_connected_services && (
                  <span className="block text-warning">
                    Includes future platform services
                  </span>
                )}
            </dd>
          </div>
          {(apiKey.allow_all_nodes || apiKey.allowed_nodes.length > 0) && (
            <div>
              <dt className="text-muted-foreground">Nodes</dt>
              <dd className="font-medium">
                {apiKey.allow_all_nodes
                  ? "All nodes, including future additions"
                  : `${apiKey.allowed_nodes.length} ${apiKey.allowed_nodes.length === 1 ? "node" : "nodes"}`}
              </dd>
            </div>
          )}
          <div>
            <dt className="text-muted-foreground">Login expiry</dt>
            <dd className="font-medium">
              {expiry ? new Date(expiry).toLocaleDateString() : "No expiry"}
            </dd>
          </div>
        </dl>
      </div>
      <details className="group border-t border-border/70 pt-1">
        <summary className="flex cursor-pointer list-none flex-wrap items-center gap-2 rounded-md py-2 text-[11px] font-medium text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
          <span>View permissions and service access</span>
          {comparison.extras.length > 0 && (
            <span className="text-warning">
              {comparison.matches ? "Matched + " : "+ "}
              {comparison.extras.length} extra
              {comparison.extras.length === 1 ? "" : "s"}
            </span>
          )}
          <ChevronDown
            aria-hidden="true"
            className="ml-auto size-3.5 transition-transform group-open:rotate-180"
          />
        </summary>
        <div className="mt-3 space-y-4">
          <AgentKeyPermissions apiKey={apiKey} />
          <AccessEntries
            label="Requested access"
            entries={comparison.matched}
          />
          <AccessEntries
            label="Access beyond the requested filters"
            entries={comparison.extras}
          />
          {children}
          <AgentKeyIssuanceNotice existing={kind === "existing"} />
        </div>
      </details>
    </section>
  );
}

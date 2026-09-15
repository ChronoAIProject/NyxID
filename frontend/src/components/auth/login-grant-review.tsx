import { KeyRound } from "lucide-react";
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
}: {
  apiKey: AgentKeySummary;
  comparison: PermissionComparison;
  kind: "existing" | "new";
}) {
  return (
    <section
      aria-label="Final access review"
      className="space-y-3 rounded-xl border border-border p-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <KeyRound className="size-4 text-muted-foreground" aria-hidden="true" />
        <h3 className="text-[12px] font-semibold">Final access review</h3>
        <Badge variant={kind === "new" ? "info" : "secondary"}>
          {kind === "new" ? "New form · creates key" : "Existing key"}
        </Badge>
      </div>
      <p className="text-[11px] text-muted-foreground">
        {kind === "new"
          ? "This draft will create one Agent Key when you approve."
          : "This approval uses the selected key; its existing secret is unchanged."}
      </p>
      <div className="rounded-lg bg-muted/20 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <strong className="text-[12px]">{apiKey.name}</strong>
          <span className="text-[11px] text-muted-foreground">
            {apiKey.owner_name}
          </span>
        </div>
        <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-[11px]">
          <dt className="text-muted-foreground">Permissions</dt>
          <dd>{effectivePermissions(apiKey.scopes)}</dd>
          <dt className="text-muted-foreground">Services</dt>
          <dd>
            {apiKey.allow_all_services
              ? "All services"
              : apiKey.allowed_services
                  .map((service) => service.name)
                  .join(", ") || "None"}
          </dd>
          <dt className="text-muted-foreground">Expiry</dt>
          <dd>
            {apiKey.expires_at
              ? new Date(apiKey.expires_at).toLocaleString()
              : "No expiry"}
          </dd>
        </dl>
      </div>
      <AccessEntries label="Requested access" entries={comparison.matched} />
      <AccessEntries
        label="Access beyond the requested filters"
        entries={comparison.extras}
      />
    </section>
  );
}

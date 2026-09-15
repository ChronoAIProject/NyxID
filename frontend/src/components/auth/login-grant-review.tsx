import { useEffect, useRef, type ReactNode } from "react";
import { ChevronDown, KeyRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { AgentKeySummary } from "@/schemas/agent-key-login";
import type {
  AccessEntry,
  PermissionComparison,
} from "@/lib/login-permissions";
import { PermissionIcon } from "./login-permission-picker";
import { AgentKeyIssuanceNotice } from "./agent-key-permissions";

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
  customize,
  actions,
  credentialExpiresAt,
}: {
  apiKey: AgentKeySummary;
  comparison: PermissionComparison;
  kind: "existing" | "new";
  children: ReactNode;
  customize: ReactNode;
  actions: ReactNode;
  credentialExpiresAt?: string;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => heading.current?.focus({ preventScroll: true }), [apiKey.id]);
  const expiry = credentialExpiresAt || apiKey.expires_at;
  return (
    <section aria-label="Authorize access" className="space-y-4">
      <div className="flex items-start gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-xl border border-border bg-muted/30">
          <KeyRound
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
        </span>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3
              ref={heading}
              tabIndex={-1}
              className="min-w-0 break-words text-[15px] font-semibold outline-none"
            >
              {apiKey.name}
            </h3>
            <Badge variant={kind === "new" ? "info" : "secondary"}>
              {kind === "new" ? "Creates new key" : "Existing key"}
            </Badge>
          </div>
          <p className="break-words text-[11px] text-muted-foreground">
            {apiKey.owner_name} ·{" "}
            {expiry
              ? `Expires ${new Date(expiry).toLocaleDateString()}`
              : "No expiry"}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2 text-[11px]">
        <p className="font-medium">Allow this device to access</p>
        <span
          className={
            comparison.extras.length ? "text-warning" : "text-muted-foreground"
          }
        >
          {comparison.matches
            ? comparison.exact
              ? "Exact match"
              : `Matched + ${comparison.extras.length} extra${comparison.extras.length === 1 ? "" : "s"}`
            : "Selection needed"}
        </span>
      </div>
      {children}
      {(apiKey.allow_all_services ||
        apiKey.allow_auto_connected_services ||
        apiKey.allow_all_nodes) && (
        <p className="text-[11px] text-warning">
          {apiKey.allow_all_services
            ? "All current and future services. "
            : apiKey.allow_auto_connected_services
              ? "Includes future platform services. "
              : ""}
          {apiKey.allow_all_nodes && "All current and future nodes."}
        </p>
      )}
      <details className="group border-t border-border">
        <summary className="flex cursor-pointer list-none items-center gap-2 py-3 text-[11px] font-medium text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
          Customize
          <ChevronDown
            aria-hidden="true"
            className="ml-auto size-3.5 transition-transform group-open:rotate-180"
          />
        </summary>
        <div className="space-y-4 pb-3">
          {customize}
          <AgentKeyIssuanceNotice existing={kind === "existing"} />
        </div>
      </details>
      {actions}
    </section>
  );
}

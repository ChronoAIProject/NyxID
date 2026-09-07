import { Badge } from "@/components/ui/badge";
import {
  effectivePermissions,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";

export function AgentKeyPermissions({ apiKey }: { apiKey: AgentKeySummary }) {
  return (
    <div className="space-y-2 text-[12px]">
      <div className="flex flex-wrap items-center gap-2">
        <strong className="break-words">{apiKey.name}</strong>
        <Badge variant={apiKey.owner_type === "org" ? "info" : "secondary"}>
          {apiKey.owner_type === "org" ? "Organization" : "Personal"}
        </Badge>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 [&_dt]:text-muted-foreground [&_dd]:break-words">
        <dt>Owner</dt>
        <dd>{apiKey.owner_name}</dd>
        {apiKey.key_prefix && (
          <>
            <dt>Prefix</dt>
            <dd className="font-mono">{apiKey.key_prefix}</dd>
          </>
        )}
        <dt>Effective permissions</dt>
        <dd>{effectivePermissions(apiKey.scopes)}</dd>
        <dt>Scopes</dt>
        <dd>{apiKey.scopes}</dd>
        <dt>Services</dt>
        <dd>
          {apiKey.allow_all_services
            ? "All services"
            : apiKey.allowed_services.map((item) => item.name).join(", ") ||
              "None"}
        </dd>
        <dt>Nodes</dt>
        <dd>
          {apiKey.allow_all_nodes
            ? "All nodes"
            : apiKey.allowed_nodes.map((item) => item.name).join(", ") ||
              "None"}
        </dd>
        <dt>Key expiry</dt>
        <dd>
          {apiKey.expires_at
            ? new Date(apiKey.expires_at).toLocaleString()
            : "No expiry"}
        </dd>
        <dt>Rate limit</dt>
        <dd>
          {apiKey.rate_limit_per_second === null
            ? "Default"
            : `${apiKey.rate_limit_per_second} requests/s`}
          ; burst {apiKey.rate_limit_burst ?? "default"}
        </dd>
        <dt>Platform</dt>
        <dd>{apiKey.platform ?? "Not specified"}</dd>
      </dl>
      {(apiKey.allow_all_services || apiKey.allow_all_nodes) && (
        <p role="note" className="text-warning">
          Unrestricted{" "}
          {apiKey.allow_all_services && apiKey.allow_all_nodes
            ? "service and node"
            : apiKey.allow_all_services
              ? "service"
              : "node"}{" "}
          access, including future resources.
        </p>
      )}
    </div>
  );
}

export function AgentKeyIssuanceNotice({ existing }: { existing: boolean }) {
  return (
    <p className="border-l-2 border-info pl-3 text-[12px] text-muted-foreground">
      A new login credential will be issued for this key.{" "}
      {existing
        ? "The key's existing secret and other consumers are unaffected. "
        : "The key's primary secret will not be shown. "}
      Revoke this login credential on the key's Login credentials section, or
      revoke the key to invalidate all its credentials.
    </p>
  );
}

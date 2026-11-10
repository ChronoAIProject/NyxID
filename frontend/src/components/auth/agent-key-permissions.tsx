import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Badge } from "@/components/ui/badge";
import {
  effectivePermissions,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";

export function AgentKeyPermissions({ apiKey }: { apiKey: AgentKeySummary }) {
  const { t } = useUiTranslation();
  return (
    <div className="space-y-2 text-12">
      <div className="flex flex-wrap items-center gap-2">
        <strong className="break-words">{apiKey.name}</strong>
        <Badge variant={apiKey.owner_type === "org" ? "info" : "secondary"}>
          {apiKey.owner_type === "org" ? t("Organization") : t("Personal")}
        </Badge>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 [&_dt]:text-muted-foreground [&_dd]:break-words">
        <dt>{t("Owner")}</dt>
        <dd>{apiKey.owner_name}</dd>
        {apiKey.key_prefix && (
          <>
            <dt>{t("Prefix")}</dt>
            <dd className="font-mono">{apiKey.key_prefix}</dd>
          </>
        )}
        <dt>{t("Effective permissions")}</dt>
        <dd>{effectivePermissions(apiKey.scopes)}</dd>
        <dt>{t("Scopes")}</dt>
        <dd>{apiKey.scopes}</dd>
        <dt>{t("Services")}</dt>
        <dd>
          {apiKey.allow_all_services
            ? t("All services")
            : [
                apiKey.allow_auto_connected_services
                  ? t("All auto-connected platform services (including future additions)")
                  : "",
                ...apiKey.allowed_services.map(
                  (item) =>
                    `${item.name}${item.auto_connected ? " (Platform)" : ""}`,
                ),
              ]
                .filter(Boolean)
                .join(", ") || t("None")}
        </dd>
        <dt>{t("Nodes")}</dt>
        <dd>
          {apiKey.allow_all_nodes
            ? t("All nodes")
            : apiKey.allowed_nodes.map((item) => item.name).join(", ") ||
              t("None")}
        </dd>
        <dt>{t("Key expiry")}</dt>
        <dd>
          {apiKey.expires_at
            ? new Date(apiKey.expires_at).toLocaleString(getFormattingLocale())
            : t("No expiry")}
        </dd>
        <dt>{t("Rate limit")}</dt>
        <dd>
          {apiKey.rate_limit_per_second === null
            ? t("Default")
            : t("{{rate_limit_per_second}} requests/s", { rate_limit_per_second: apiKey.rate_limit_per_second })}{t("; burst ")}{apiKey.rate_limit_burst ?? t("default")}
        </dd>
        <dt>{t("Platform")}</dt>
        <dd>{apiKey.platform ?? t("Not specified")}</dd>
      </dl>
      {(apiKey.allow_all_services || apiKey.allow_all_nodes) && (
        <p role="note" className="text-warning">{t("Unrestricted")}{" "}
          {apiKey.allow_all_services && apiKey.allow_all_nodes
            ? t("service and node")
            : apiKey.allow_all_services
              ? t("service")
              : t("node")}{" "}{t("access, including future resources.")}</p>
      )}
    </div>
  );
}

export function AgentKeyIssuanceNotice({ existing }: { existing: boolean }) {
  const { t } = useUiTranslation();
  return (
    <p className="border-l-2 border-info pl-3 text-12 text-muted-foreground">{t("A new login credential will be issued for this key.")}{" "}
      {existing
        ? t("The key's existing secret and other consumers are unaffected. ")
        : t("The key's primary secret will not be shown. ")}{t("Revoke this login credential on the key's Login credentials section, or revoke the key to invalidate all its credentials.")}</p>
  );
}

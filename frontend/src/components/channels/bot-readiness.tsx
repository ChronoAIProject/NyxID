import { routeAgentIssue } from "@/lib/channel-readiness";
import { formatDate } from "@/lib/utils";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import type { ApiKey } from "@/types/api";
import type {
  BotVerification,
  ChannelConversationItem,
} from "@/types/channels";

export function CredentialVerification({
  check,
  pending,
  error,
  previousCheckId,
}: {
  readonly check: BotVerification | null | undefined;
  readonly pending: boolean;
  readonly error: string | null;
  readonly previousCheckId: string | null;
}) {
  const currentError = check && check.id !== previousCheckId ? null : error;
  const status = pending ? "pending" : currentError ? "failed" : check?.status;
  const label =
    status === "verified"
      ? "Credentials verified"
      : status === "pending"
        ? "Checking credentials…"
        : status === "failed"
          ? "Credential check failed"
          : status === "incomplete"
            ? "Credential check incomplete"
            : "Credentials not checked";
  return (
    <DetailSection title="Credential Verification">
      <div
        className="space-y-3 p-4 text-[12px] text-muted-foreground"
        aria-live="polite"
      >
        <Badge
          variant={
            status === "verified"
              ? "success"
              : status === "failed"
                ? "destructive"
                : "warning"
          }
        >
          {label}
        </Badge>
        {!pending && (currentError || check?.message) && (
          <ErrorBanner message={currentError ?? check?.message ?? ""} />
        )}
        {!pending && !currentError && check && (
          <p>
            Last check: {formatDate(check.completed_at ?? check.started_at)}
          </p>
        )}
        <p>
          Verify Bot checks access to this WhatsApp phone through Meta. Webhook
          subscriptions, an agent route, and a real message exchange must be
          checked separately.
        </p>
      </div>
    </DetailSection>
  );
}

export function RouteReadiness({
  keys,
  ownerOrgId,
  ownerLabel,
  conversations,
}: {
  readonly keys: readonly ApiKey[];
  readonly ownerOrgId: string | null;
  readonly ownerLabel: string;
  readonly conversations?: readonly ChannelConversationItem[];
}) {
  const eligible = keys.filter((key) => !routeAgentIssue(key, ownerOrgId));
  const readyRoutes = conversations?.filter(
    (route) =>
      route.is_active &&
      eligible.some((key) => key.id === route.agent_api_key_id),
  );
  const message =
    keys.length === 0
      ? "No agent keys exist in this owner scope."
      : eligible.length === 0
        ? "No eligible agent key is configured in this owner scope."
        : conversations && !readyRoutes?.length
          ? "No active route has an eligible agent key."
          : conversations
            ? "An agent route is configured. Confirm callback acceptance with a real incoming message."
            : "Select a key registered with your agent runtime.";
  return (
    <div className="space-y-2 rounded-lg border border-border bg-muted/50 p-3 text-[12px] text-muted-foreground">
      <p className="font-medium text-foreground">Agent setup · {ownerLabel}</p>
      <p>{message}</p>
      <p>
        Register the intended agent runtime with a key owned by {ownerLabel}.
        The runtime must recognize that key, authenticate NyxID callbacks, and
        send asynchronous replies. Setting a callback URL alone does not
        register the runtime.
      </p>
      {keys
        .filter((key) => routeAgentIssue(key, ownerOrgId) === "No callback URL")
        .map((key) => (
          <p key={key.id}>
            <a
              className="underline"
              href={`/keys/api-key/${encodeURIComponent(key.id)}`}
            >
              Configure {key.name}
            </a>
            : callback URL missing.
          </p>
        ))}
      {keys.length === 0 && (
        <p>
          <a className="underline" href="/keys?tab=nyxid">
            Open Agent Keys
          </a>{" "}
          and choose {ownerLabel} as the owner when creating a runtime key.
        </p>
      )}
      {keys.some((key) =>
        ["Expired", "Inactive", "Assistant chat key"].includes(
          routeAgentIssue(key, ownerOrgId) ?? "",
        ),
      ) && (
        <p>
          Expired, inactive, and assistant chat keys cannot receive channel
          messages.
        </p>
      )}
    </div>
  );
}

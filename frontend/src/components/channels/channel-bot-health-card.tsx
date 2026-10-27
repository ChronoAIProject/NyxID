import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  useChannelBotHealth,
  useResubscribeChannelBot,
  useVerifyChannelBot,
} from "@/hooks/use-channel-bots";
import type { ChannelBotHealth } from "@/schemas/channel-bot-health";

function when(value: string | null): string {
  if (!value) return "never";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function errorLine(
  error: ChannelBotHealth["diagnostics"]["last_outbound_error"],
): string | null {
  if (!error) return null;
  return `${error.reason ?? `error ${error.code}`} at ${when(error.at)}`;
}

function Row({
  label,
  value,
}: {
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="flex justify-between gap-3 text-12">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-right">{value}</span>
    </div>
  );
}

/** Delivery diagnostics and the server's recovery recommendation. */
export function ChannelBotHealthCard({ botId }: { readonly botId: string }) {
  const health = useChannelBotHealth(botId);
  const resubscribe = useResubscribeChannelBot();
  const verify = useVerifyChannelBot({ creditsOwnerId: health.data?.user_id });
  if (health.isLoading)
    return <p className="text-12 text-muted-foreground">Checking health…</p>;
  if (health.error || !health.data)
    return (
      <p role="alert" className="text-12 text-destructive">
        Health is unavailable
        {health.error instanceof Error ? `: ${health.error.message}` : "."}
      </p>
    );
  const data = health.data;
  const d = data.diagnostics;
  const outbound = errorLine(d.last_outbound_error);
  const webhook = errorLine(d.last_webhook_error);
  const healthy = data.recommended_action === "none";
  const action =
    data.can_write && data.recommended_action === "resubscribe"
      ? {
          label: "Resubscribe",
          run: () =>
            resubscribe.mutate(botId, {
              onSuccess: (result) =>
                result.recommended_action === "none"
                  ? toast.success("Subscription registered")
                  : toast.warning(result.recommended_action_text),
            }),
          pending: resubscribe.isPending,
        }
      : data.can_write && data.recommended_action === "verify"
        ? {
            label: "Verify",
            run: () =>
              verify.mutate(botId, {
                onSettled: () => void health.refetch(),
              }),
            pending: verify.isPending,
          }
        : null;
  return (
    <section
      aria-label="Channel health"
      className="space-y-2 rounded-md border border-border p-3"
    >
      <div className="flex items-center gap-2 text-13 font-medium">
        Health
        <Badge variant={healthy ? "success" : "warning"}>
          {healthy ? "Healthy" : "Needs attention"}
        </Badge>
        <Badge variant="secondary">
          {data.mode === "poll" ? "Polling" : "Webhook"}
        </Badge>
      </div>
      <Row
        label="Last inbound"
        value={`${when(d.last_inbound_at)}${d.last_inbound_source ? ` (${d.last_inbound_source})` : ""}`}
      />
      {data.platform === "discord" && (
        <Row
          label="Last NyxID relay Gateway state"
          value={d.discord_gateway
            ? `${d.discord_gateway.state}${d.discord_gateway.reason ? ` (${d.discord_gateway.reason})` : ""}, reported ${when(d.discord_gateway.updated_at)}`
            : "No local connection recorded. Relay chat requires channel:discord-gateway and Discord Message Content Intent. Gateway-hosted bots use the Agent Event Gateway connection."}
        />
      )}
      <Row label="Last reply sent" value={when(d.last_outbound_at)} />
      {outbound && <Row label="Last reply error" value={outbound} />}
      {webhook && <Row label="Last webhook error" value={webhook} />}
      {d.subscription_state && (
        <Row
          label="Subscription"
          value={`${d.subscription_state.registered ? "registered" : `missing${d.subscription_state.last_error ? ` (${d.subscription_state.last_error})` : ""}`}, checked ${when(d.subscription_state.checked_at)}`}
        />
      )}
      {d.failure_reason && <Row label="Failure" value={d.failure_reason} />}
      {d.consecutive_errors > 0 && (
        <Row label="Consecutive errors" value={String(d.consecutive_errors)} />
      )}
      {data.next_poll_at && (
        <Row label="Next poll" value={when(data.next_poll_at)} />
      )}
      <p
        className={
          healthy ? "text-12 text-muted-foreground" : "text-12 text-warning"
        }
      >
        {data.recommended_action_text}
      </p>
      {action && (
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={action.pending}
          onClick={action.run}
        >
          {action.label}
        </Button>
      )}
      {resubscribe.error && (
        <p role="alert" className="text-12 text-destructive">
          {resubscribe.error.message}
        </p>
      )}
    </section>
  );
}

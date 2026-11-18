import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
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
  if (!value) return translateUi("never");
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString(getFormattingLocale());
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
  useUiTranslation();
  return (
    <div className="flex justify-between gap-3 text-14">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-right">{value}</span>
    </div>
  );
}

/** Delivery diagnostics and the server's recovery recommendation. */
export function ChannelBotHealthCard({ botId }: { readonly botId: string }) {
  useUiTranslation();
  const health = useChannelBotHealth(botId);
  const resubscribe = useResubscribeChannelBot();
  const verify = useVerifyChannelBot({ creditsOwnerId: health.data?.user_id });
  if (health.isLoading)
    return <p className="text-14 text-muted-foreground">{translateUi("Checking health…")}</p>;
  if (health.error || !health.data)
    return (
      <p role="alert" className="text-14 text-destructive">{translateUi("Health is unavailable")}{health.error instanceof Error ? `: ${health.error.message}` : "."}
      </p>
    );
  const data = health.data;
  const d = data.diagnostics;
  const outbound = errorLine(d.last_outbound_error);
  const webhook = errorLine(d.last_webhook_error);
  const healthy = data.recommended_action === "none";
  const healthyLabel = data.platform === "x"
    ? d.last_inbound_outcome?.code === "x_reply_sent"
      ? translateUi("Reply accepted")
      : translateUi("No problem detected")
    : translateUi("Healthy");
  const inboundOutcomes = Object.entries(d.inbound_outcomes ?? {});
  const action =
    data.can_write && data.recommended_action === "resubscribe"
      ? {
          label: "Resubscribe",
          run: () =>
            resubscribe.mutate(botId, {
              onSuccess: (result) =>
                result.recommended_action === "none"
                  ? toast.success(translateUi("Subscription registered"))
                  : toast.warning(translateUi(result.recommended_action_text)),
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
      aria-label={translateUi("Channel health")}
      className="space-y-2 rounded-md border border-border p-3"
    >
      <div className="flex items-center gap-2 type-card-title">{translateUi("Health")}<Badge variant={healthy ? "success" : "warning"}>
          {healthy ? healthyLabel : translateUi("Needs attention")}
        </Badge>
        <Badge variant="secondary">
          {data.platform === "discord" ? (data.discord_intake?.receiving_side === "interactions_only" ? translateUi("Interactions") : translateUi("Gateway")) : data.mode === "poll" ? translateUi("Polling") : translateUi("Webhook")}
        </Badge>
      </div>
      {data.discord_intake && <>
        <Row label={translateUi("Receiving side")} value={data.discord_intake.receiving_side === "native_gateway" ? "Agent Event Gateway" : data.discord_intake.receiving_side === "interactions_only" ? "Discord interactions" : "NyxID relay"} />
        <Row label={translateUi("Intake evidence")} value={`${data.discord_intake.state}${data.discord_intake.code ? ` (${data.discord_intake.code})` : ""}`} />
        <Row label={translateUi("Last confirmed inbound")} value={when(data.discord_intake.last_inbound_at)} />
        {data.discord_intake.handed_over_at && <Row label={translateUi("Handed over")} value={when(data.discord_intake.handed_over_at)} />}
        {data.discord_intake.block_reason && <Row label={translateUi("Intake blocked by")} value={data.discord_intake.block_reason} />}
      </>}
      <Row
        label={translateUi("Last inbound")}
        value={`${when(d.last_inbound_at)}${d.last_inbound_source ? ` (${d.last_inbound_source})` : ""}`}
      />
      {data.platform === "discord" && (
        <Row
          label={translateUi("Last NyxID relay Gateway state")}
          value={d.discord_gateway
            ? `${d.discord_gateway.state}${d.discord_gateway.reason ? ` (${d.discord_gateway.reason})` : ""}, reported ${when(d.discord_gateway.updated_at)}`
            : "No local connection recorded. Relay chat requires channel:discord-gateway and Discord Message Content Intent. Gateway-hosted bots use the Agent Event Gateway connection."}
        />
      )}
      <Row label={translateUi("Last outbound attempt")} value={when(d.last_outbound_at)} />
      {d.last_inbound_outcome && (
        <Row
          label={translateUi("Last inbound outcome")}
          value={translateUi("{{code}} at {{time}}", {
            code: d.last_inbound_outcome.code,
            time: when(d.last_inbound_outcome.at),
          })}
        />
      )}
      {inboundOutcomes.length > 0 && (
        <details>
          <summary className="cursor-pointer type-secondary text-muted-foreground">
            {translateUi("Inbound diagnostics")}
          </summary>
          <div className="space-y-2 pt-2">
            {inboundOutcomes.map(([code, outcome]) => (
              <Row
                key={code}
                label={code}
                value={translateUi("{{count}} observed; last {{time}}", {
                  count: outcome.count,
                  time: when(outcome.at),
                })}
              />
            ))}
          </div>
        </details>
      )}
      {outbound && <Row label={translateUi("Last reply error")} value={outbound} />}
      {d.last_outbound_notice?.reason === "telegram_formatting_fallback" && (
        <Row
          label={translateUi("Formatting notice")}
          value={translateUi("Telegram rejected formatting at {{time}}. NyxID attempted the same text without styling; see the delivery result for confirmation.", { time: when(d.last_outbound_notice.at) })}
        />
      )}
      {webhook && <Row label={translateUi("Last webhook error")} value={webhook} />}
      {d.subscription_state && (
        <Row
          label={translateUi("Subscription")}
          value={`${d.subscription_state.registered ? "registered" : `missing${d.subscription_state.last_error ? ` (${d.subscription_state.last_error})` : ""}`}, checked ${when(d.subscription_state.checked_at)}`}
        />
      )}
      {d.failure_reason && <Row label={translateUi("Failure")} value={d.failure_reason} />}
      {d.consecutive_errors > 0 && (
        <Row label={translateUi("Consecutive errors")} value={String(d.consecutive_errors)} />
      )}
      {data.next_poll_at && (
        <Row label={translateUi("Next poll")} value={when(data.next_poll_at)} />
      )}
      <p
        className={
          healthy ? "text-14 text-muted-foreground" : "text-14 text-warning"
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
        <p role="alert" className="text-14 text-destructive">
          {resubscribe.error.message}
        </p>
      )}
    </section>
  );
}

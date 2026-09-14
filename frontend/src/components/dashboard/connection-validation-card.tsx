import { useEffect, useState } from "react";
import { useValidateKey } from "@/hooks/use-keys";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { ServiceValidationResponse } from "@/schemas/service-validation";

const outcomeLabels: Record<ServiceValidationResponse["outcome"], string> = {
  authenticated: "Connection checked",
  permission_denied: "Permission required",
  credential_rejected: "Credential rejected",
  configuration_error: "Configuration needs attention",
  billing_blocked: "Provider billing blocked",
  rate_limited: "Provider rate limit reached",
  transport_unknown: "Could not check connection",
  unsupported: "Check unavailable",
};

const reasonMessages: Readonly<Record<string, string>> = {
  node_agent_upgrade_required:
    "Upgrade the connected node agent to check this connection.",
  credential_unavailable:
    "The stored credential could not be used. Reconnect this service.",
  attempt_superseded:
    "The connection changed while it was being checked. Check again.",
};

export function ConnectionValidationCard({
  serviceId,
  active,
}: {
  readonly serviceId: string;
  readonly active: boolean;
}) {
  const validation = useValidateKey(serviceId);
  const [now, setNow] = useState(Date.now);
  const result = validation.data;
  useEffect(() => {
    if (!result) return;
    const timer = window.setTimeout(
      () => setNow(Date.now()),
      Math.max(0, Date.parse(result.valid_until) - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [result]);
  const expired = result && Date.parse(result.valid_until) <= now;

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-[15px]">Connection validation</CardTitle>
        <CardDescription>
          Check whether your provider accepts this connection. Provider rate
          limits apply. Read access shows the key; validation requires proxy
          permission.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3 text-[12px]">
        <Button
          onClick={() => validation.mutate({ force: Boolean(result) })}
          disabled={!active}
          isLoading={validation.isPending}
        >
          Check connection
        </Button>
        {!active && (
          <p className="text-muted-foreground">
            Enable this service before checking its connection.
          </p>
        )}
        <div aria-live="polite" className="space-y-2">
          {result && (
            <>
              <Badge
                variant={
                  expired
                    ? "secondary"
                    : result.outcome === "authenticated"
                      ? "success"
                      : "warning"
                }
              >
                {expired ? "Check expired" : outcomeLabels[result.outcome]}
              </Badge>
              <p className="text-muted-foreground">{result.claim}</p>
              {reasonMessages[result.reason_code] && (
                <p className="text-muted-foreground">
                  {reasonMessages[result.reason_code]}
                </p>
              )}
              <p className="text-[11px] text-muted-foreground">
                Checked {new Date(result.checked_at).toLocaleString()}. Results
                describe that check and expire after five minutes.
              </p>
            </>
          )}
          {validation.error && (
            <p role="alert" className="text-destructive">
              {validation.error.message}
            </p>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { AppConnectItem } from "@/schemas/app-connect-links";

const stateLabels: Record<AppConnectItem["state"], string> = {
  unmet: "Not connected",
  connecting: "Connecting",
  reauthorizing: "Reauthorizing",
  validating: "Checking",
  met: "Ready",
  unknown: "Check needed",
  failed: "Needs attention",
  skipped: "Skipped",
};
const readinessCopy: Partial<Record<AppConnectItem["readiness"], string>> = {
  disabled: "Disabled, enable to use.",
  needs_reauth: "Reconnect to grant the required provider permissions.",
  unsatisfiable: "This requirement is unavailable. Contact the app developer.",
  broken: "This connection needs to be repaired.",
};
const reasonCopy: Record<string, string> = {
  slug_shadowed:
    "This account shares its name with another of your connections. Rename one of them, or choose the other, to use it here.",
  credential_unavailable:
    "The stored credential could not be used. Reconnect this service.",
  attempt_superseded:
    "The connection changed while it was being checked. Check again.",
  node_agent_upgrade_required:
    "Update the credential node agent to check this connection.",
  provider_authorization_failed:
    "Provider authorization failed. Connect again to retry.",
  attempt_interrupted: "The check was interrupted. Check again.",
  validation_unavailable: "The connection could not be checked. Try again.",
  child_cancelled:
    "Connection cancelled. Connect or choose an account to try again.",
};

export function AppConnectChecklistItem({
  item,
  now,
  pending,
  onConnect,
  onSelect,
  onValidate,
  children,
}: {
  readonly item: AppConnectItem;
  readonly now: number;
  readonly pending: boolean;
  readonly onConnect: (slug: string, reauthorize: boolean) => void;
  readonly onSelect: (id: string | null) => void;
  readonly onValidate: () => void;
  readonly children?: React.ReactNode;
}) {
  const [showChoices, setShowChoices] = useState(false);
  const [slug, setSlug] = useState(item.catalog_slugs[0] ?? "");
  const included = item.readiness === "included" && item.state === "met";
  const selectedCatalogSlug = item.choices.find(
    (c) => c.user_service_id === item.user_service_id,
  )?.catalog_slug;
  const expired =
    item.state === "met" &&
    !!item.valid_until &&
    new Date(item.valid_until).getTime() <= now;
  const label = included
    ? "Included"
    : expired
      ? "Check expired"
      : item.readiness === "disabled"
        ? "Disabled"
        : stateLabels[item.state];
  const variant =
    included || (item.state === "met" && !expired)
      ? "success"
      : item.state === "failed"
        ? "destructive"
        : "warning";
  const startingOver =
    item.state === "connecting" || item.state === "reauthorizing";
  const disabled =
    pending ||
    (item.readiness === "unsatisfiable" &&
      item.reason_code !== "slug_shadowed");
  const executionDisabled = disabled || item.readiness === "disabled";
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-3 pb-3">
        <CardTitle>
          {item.label}{" "}
          {item.optional && (
            <span className="font-normal text-muted-foreground">
              (optional)
            </span>
          )}
        </CardTitle>
        <Badge variant={variant}>{label}</Badge>
      </CardHeader>
      <CardContent className="space-y-3 text-[12px]">
        {item.slug && item.state !== "skipped" && (
          <p className="font-medium">{item.slug}</p>
        )}
        {item.claim && <p className="text-muted-foreground">{item.claim}</p>}
        {item.reason_code && reasonCopy[item.reason_code] ? (
          <p className="text-muted-foreground">
            {reasonCopy[item.reason_code]}
          </p>
        ) : (
          readinessCopy[item.readiness] && (
            <p className="text-muted-foreground">
              {readinessCopy[item.readiness]}
            </p>
          )
        )}
        {item.readiness === "disabled" && item.user_service_id && (
          <a
            href={`/keys/${encodeURIComponent(item.user_service_id)}`}
            target="_blank"
            rel="noopener noreferrer"
            className="font-medium underline underline-offset-4"
          >
            Manage connection
          </a>
        )}
        {item.readiness === "needs_reauth" &&
          item.required_scopes.length > 0 && (
            <p className="text-muted-foreground">
              Required permissions: {item.required_scopes.join(", ")}
            </p>
          )}
        {item.validated_at && (
          <p className="text-[11px] text-muted-foreground">
            Checked {new Date(item.validated_at).toLocaleString()}
          </p>
        )}
        {!included && (
          <>
            {showChoices && (
              <div className="space-y-2">
                {item.choices.length > 0 && (
                  <Select
                    onValueChange={(id) => {
                      onSelect(id);
                      setShowChoices(false);
                    }}
                    disabled={disabled}
                  >
                    <SelectTrigger
                      aria-label={`Choose an account for ${item.label}`}
                    >
                      <SelectValue placeholder="Choose a connected account" />
                    </SelectTrigger>
                    <SelectContent>
                      {item.choices.map((choice) => (
                        <SelectItem
                          key={choice.user_service_id}
                          value={choice.user_service_id}
                        >
                          {choice.slug}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
                <Select
                  value={slug}
                  onValueChange={setSlug}
                  disabled={disabled}
                >
                  <SelectTrigger
                    aria-label={`Choose a service for ${item.label}`}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {item.catalog_slugs.map((value) => (
                      <SelectItem key={value} value={value}>
                        {value}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  disabled={disabled || !slug}
                  onClick={() => onConnect(slug, false)}
                >
                  Connect another account
                </Button>
              </div>
            )}
            <div className="flex flex-wrap justify-end gap-2">
              <Button
                variant="ghost"
                disabled={disabled}
                onClick={() => setShowChoices((v) => !v)}
              >
                Change
              </Button>
              {item.optional && item.state !== "skipped" && (
                <Button
                  variant="ghost"
                  disabled={disabled}
                  onClick={() => onSelect(null)}
                >
                  Skip
                </Button>
              )}
              {item.user_service_id && (
                <Button
                  disabled={
                    executionDisabled || item.reason_code === "slug_shadowed"
                  }
                  isLoading={pending && item.state === "validating"}
                  onClick={onValidate}
                >
                  Re-check
                </Button>
              )}
              {item.readiness === "needs_reauth" ? (
                <Button
                  variant="primary"
                  disabled={executionDisabled || !selectedCatalogSlug}
                  onClick={() =>
                    selectedCatalogSlug && onConnect(selectedCatalogSlug, true)
                  }
                >
                  {startingOver ? "Start over" : "Reauthorize"}
                </Button>
              ) : (
                (item.state === "unmet" ||
                  item.state === "failed" ||
                  startingOver ||
                  item.state === "skipped") && (
                  <Button
                    variant="primary"
                    disabled={executionDisabled || !slug}
                    onClick={() =>
                      item.catalog_slugs.length > 1
                        ? setShowChoices(true)
                        : onConnect(slug, false)
                    }
                  >
                    {startingOver ? "Start over" : "Connect"}
                  </Button>
                )
              )}
            </div>
            {children}
          </>
        )}
      </CardContent>
    </Card>
  );
}

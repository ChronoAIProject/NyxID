import { Badge } from "@/components/ui/badge";
import { useAdminBillingService } from "@/hooks/use-admin-billing";
import { metricLabel } from "@/schemas/billing-metrics";
import type {
  BillingService,
  BillingSource,
  CredentialClass,
  EffectiveClass,
  EffectivePrice,
  EffectiveReason,
} from "@/schemas/admin-billing";

const SOURCE_LABELS: Record<BillingSource, string> = {
  env: "Environment",
  global: "Global default",
  service: "Service",
  legacy: "Legacy",
  free: "Free",
};

const SOURCE_VARIANTS: Record<
  BillingSource,
  "info" | "accent" | "default" | "warning" | "secondary"
> = {
  env: "info",
  global: "accent",
  service: "default",
  legacy: "warning",
  free: "secondary",
};

const CLASS_LABELS: Record<CredentialClass, string> = {
  nyxid_managed_master: "NyxID platform key",
  nyxid_platform_oauth_app: "NyxID shared OAuth app",
  user_owned: "Own key",
  agent_override_user_owned: "Agent credential override",
  node_managed: "Node credential",
  no_auth: "No credential",
};

const REASON_TEXT: Record<EffectiveReason, string> = {
  billing_disabled:
    "Billing is disabled for this deployment (BILLING_ENABLED).",
  nyxid_credentials_only:
    "Free: this service charges only NyxID credentials and the shared OAuth app.",
  lane_synced: "The lane price is synced and charged.",
  lane_unsynced_legacy:
    "The lane price is not synced yet; legacy platform billing charges meanwhile.",
  lane_unsynced_free:
    "The lane price is not synced yet and legacy billing is off, so it is free meanwhile.",
  lane_not_configured: "Free: no price is configured for this credential lane.",
  no_auth:
    "Free: traffic without a credential is never priced once lanes exist.",
  legacy_platform_billable: "Legacy platform billing charges the legacy price.",
  not_billable: "Free: no lane prices and legacy platform billing is off.",
};

const ORIGIN_LABELS: Record<EffectivePrice["origin"], string> = {
  lane_primary: "lane price",
  lane_component: "lane component",
  operation: "operation price",
  service_price: "legacy service price",
  legacy_plan: "Lago plan price",
  resale_plan: "resale",
};

export function SourceBadge({ source }: { readonly source: BillingSource }) {
  return (
    <Badge variant={SOURCE_VARIANTS[source]} title={`Source: ${source}`}>
      {SOURCE_LABELS[source]}
    </Badge>
  );
}

function SyncBadge({
  status,
}: {
  readonly status: EffectivePrice["sync_status"];
}) {
  if (!status) return null;
  return (
    <Badge variant={status === "synced" ? "success" : "warning"}>
      {status === "synced"
        ? "Synced"
        : status === "failed"
          ? "Failed"
          : "Pending"}
    </Badge>
  );
}

function priceLabel(price: EffectivePrice): string {
  const unit = price.operation
    ? `${price.operation} request`
    : metricLabel(price.metric, 1);
  const amount =
    price.credits_per_unit === null
      ? "rate not cached"
      : `${price.credits_per_unit} credits / ${unit}`;
  return `${amount} · ${ORIGIN_LABELS[price.origin]}`;
}

export function PriceLine({ price }: { readonly price: EffectivePrice }) {
  return (
    <li className="flex flex-wrap items-center gap-1.5 text-12">
      <span>{priceLabel(price)}</span>
      <SourceBadge source={price.source} />
      <SyncBadge status={price.sync_status} />
      {price.cached_rate?.stale && <Badge variant="warning">Stale rate</Badge>}
      {price.cached_rate?.retired && (
        <Badge variant="destructive">Retired rate</Badge>
      )}
      {price.sync_error && (
        <span className="text-11 text-destructive">{price.sync_error}</span>
      )}
    </li>
  );
}

export function EffectiveClassBlock({
  group,
}: {
  readonly group: EffectiveClass;
}) {
  return (
    <div className="space-y-1" data-testid="effective-class">
      <div className="flex flex-wrap items-center gap-1.5 text-12 font-medium">
        <span>
          {group.credential_classes.map((c) => CLASS_LABELS[c]).join(", ")}
        </span>
        <SourceBadge source={group.source} />
        {group.platform_charged ? (
          <Badge variant="default">Charged</Badge>
        ) : (
          <Badge variant="secondary">Not charged</Badge>
        )}
      </div>
      <p className="text-11 text-muted-foreground">
        {REASON_TEXT[group.reason]}
      </p>
      {group.prices.length > 0 && (
        <ul className="space-y-1">
          {group.prices.map((price) => (
            <PriceLine
              key={`${price.layer}:${price.lago_metric_code}`}
              price={price}
            />
          ))}
        </ul>
      )}
      {group.pending.length > 0 && (
        <div className="space-y-1">
          <div className="text-11 text-muted-foreground">
            Configured, not charged until synced:
          </div>
          <ul className="space-y-1">
            {group.pending.map((price) => (
              <PriceLine
                key={`pending:${price.lago_metric_code}:${price.operation ?? ""}`}
                price={price}
              />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

export function EffectiveBilling({
  service,
}: {
  readonly service: BillingService;
}) {
  const { resale } = service.effective;
  return (
    <div className="space-y-3">
      {service.effective.classes.map((group) => (
        <EffectiveClassBlock
          key={group.credential_classes.join(",")}
          group={group}
        />
      ))}
      {resale.configured && (
        <p className="flex items-center gap-1.5 text-11 text-muted-foreground">
          Resale{" "}
          {resale.applied
            ? "applies to NyxID platform-key traffic"
            : "is configured but disabled"}
          <SourceBadge source={resale.source} />
        </p>
      )}
    </div>
  );
}

/** Server-resolved effective billing, shown inside the service editor. */
export function EffectiveBillingSummary({
  serviceId,
}: {
  readonly serviceId: string;
}) {
  const { data, isLoading, error } = useAdminBillingService(serviceId);
  return (
    <section className="space-y-2 rounded-lg border border-border/50 p-3">
      <h3 className="text-13 font-semibold">Effective billing</h3>
      {isLoading && (
        <p className="text-12 text-muted-foreground">Resolving prices…</p>
      )}
      {error && (
        <p className="text-12 text-destructive">
          Effective billing is unavailable: {error.message}
        </p>
      )}
      {data && <EffectiveBilling service={data} />}
      <p className="text-11 text-muted-foreground">
        Resolved by the server from the saved configuration exactly as billing
        charges it. Global defaults live in Admin → Billing.
      </p>
    </section>
  );
}

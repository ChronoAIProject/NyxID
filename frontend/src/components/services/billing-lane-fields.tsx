import { useFormContext } from "react-hook-form";
import { useEndpoints } from "@/hooks/use-endpoints";
import { SERVICE_BILLING_METRICS, metricLabel } from "@/schemas/billing-metrics";
import type { SharedServiceFormData } from "@/schemas/services";
import type { DownstreamService } from "@/types/api";
import { Badge } from "@/components/ui/badge";
import {
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

/** The form fields the lane editor reads and writes. */
export type BillingLaneFormData = Pick<
  SharedServiceFormData,
  "inference" | "byok_pricing" | "platform_key_pricing"
>;

/**
 * BYOK and platform-key lane prices for one catalog service. Shared by the
 * service editor and the central admin billing page; the parent form owns
 * submission.
 */
export function BillingLaneFields({
  service,
}: {
  readonly service?: Pick<
    DownstreamService,
    "id" | "billing" | "declared_operations" | "x_channel_billing"
  >;
}) {
  const form = useFormContext<BillingLaneFormData>();
  const inference = form.watch("inference");
  const suggestedMetrics = SERVICE_BILLING_METRICS.filter(
    (metric) =>
      metric !== "voice_seconds" ||
      inference?.voice?.billing_metrics.includes(metric),
  );
  const { data: endpoints = [] } = useEndpoints(service?.id ?? "");
  // Active endpoints, declared channel operations, then any saved key that is
  // no longer listed, so a stale price can still be cleared.
  const operationChoices = (
    prices: readonly { readonly operation: string }[] = [],
  ) => {
    const choices = new Map<string, string>();
    for (const endpoint of endpoints)
      if (endpoint.is_active) choices.set(endpoint.name, endpoint.name);
    for (const declared of service?.declared_operations ?? [])
      choices.set(declared.operation, declared.label);
    for (const price of prices)
      if (!choices.has(price.operation))
        choices.set(price.operation, price.operation);
    return [...choices].map(([operation, label]) => ({ operation, label }));
  };
  return (
    <section className="space-y-3">
      <h3 className="text-13 font-semibold">Billing lanes</h3>
      {service?.x_channel_billing && (
        <p className="text-12 text-muted-foreground">
          X channels bill through the {service.x_channel_billing.lane} lane.
          Configure a synced Requests price on that lane to enable paid X
          channels.
        </p>
      )}
      {!form.watch("byok_pricing") && !form.watch("platform_key_pricing") && (
        <p className="text-xs text-muted-foreground">
          No lane prices are configured. Legacy billing below still applies.
        </p>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
        {(["byok_pricing", "platform_key_pricing"] as const).map((field) => {
          const lane = form.watch(field);
          const status = service?.billing?.[field]?.sync_status;
          const setLane = (value: BillingLaneFormData[typeof field]) =>
            form.setValue(field, value, {
              shouldDirty: true,
              shouldValidate: true,
            });
          return (
            <div
              key={field}
              className="space-y-3 rounded-lg border border-border/50 p-3"
            >
              <div className="text-xs font-medium">
                {field === "byok_pricing"
                  ? "Your own key"
                  : "NyxID platform key"}
              </div>
              <div className="flex items-center justify-between">
                <Label htmlFor={`${field}-enabled`}>
                  {lane ? "Charge" : "Free"}
                </Label>
                <Switch
                  id={`${field}-enabled`}
                  checked={Boolean(lane)}
                  onCheckedChange={(enabled) =>
                    setLane(
                      enabled
                        ? { metric: "requests", credits_per_unit: "0" }
                        : null,
                    )
                  }
                />
              </div>
              {lane && (
                <>
                  <Label htmlFor={`${field}-metric`}>Unit</Label>
                  <Select
                    value={lane.metric}
                    onValueChange={(metric) =>
                      setLane({
                        ...lane,
                        metric: metric as NonNullable<
                          BillingLaneFormData[typeof field]
                        >["metric"],
                        // Operation prices exist only on Requests lanes.
                        ...(metric === "requests" ? {} : { operations: [] }),
                      })
                    }
                  >
                    <SelectTrigger id={`${field}-metric`}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SERVICE_BILLING_METRICS.filter(
                        (metric) =>
                          suggestedMetrics.includes(metric) ||
                          metric === lane.metric,
                      ).map((metric) => (
                        <SelectItem key={metric} value={metric}>
                          {metricLabel(metric)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormField
                    control={form.control}
                    name={`${field}.credits_per_unit`}
                    render={({ field: input }) => (
                      <FormItem>
                        <FormLabel>Credits per unit</FormLabel>
                        <FormControl>
                          <Input {...input} inputMode="decimal" />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  {lane.metric === "requests" && (
                    <OperationPrices
                      operations={operationChoices(lane.operations ?? [])}
                      prices={lane.operations ?? []}
                      saved={service?.billing?.[field]?.operations ?? []}
                      errorAt={(index) =>
                        form.formState.errors[field]?.operations?.[index]
                          ?.credits_per_unit?.message
                      }
                      onChange={(operations) =>
                        setLane({ ...lane, operations })
                      }
                    />
                  )}
                  {status && (
                    <Badge
                      variant={status === "synced" ? "success" : "warning"}
                    >
                      {status === "synced"
                        ? "Synced"
                        : status === "failed"
                          ? "Failed"
                          : "Pending"}
                    </Badge>
                  )}
                  {(lane.components ?? []).map((component, index) => {
                    const saved = service?.billing?.[field]?.components?.find(
                      (item) => item.metric === component.metric,
                    );
                    return (
                      <div
                        key={index}
                        className="space-y-2 border-t border-border/50 pt-3"
                      >
                        <FormField
                          control={form.control}
                          name={`${field}.components.${index}.metric`}
                          render={({ field: input }) => (
                            <FormItem>
                              <FormLabel>Component unit</FormLabel>
                              <Select
                                value={input.value}
                                onValueChange={input.onChange}
                              >
                                <FormControl>
                                  <SelectTrigger
                                    aria-label={`Component ${index + 1} unit`}
                                  >
                                    <SelectValue />
                                  </SelectTrigger>
                                </FormControl>
                                <SelectContent>
                                  {SERVICE_BILLING_METRICS.filter(
                                    (metric) =>
                                      suggestedMetrics.includes(metric) ||
                                      metric === input.value,
                                  ).map((metric) => (
                                    <SelectItem key={metric} value={metric}>
                                      {metricLabel(metric)}
                                    </SelectItem>
                                  ))}
                                </SelectContent>
                              </Select>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={form.control}
                          name={`${field}.components.${index}.credits_per_unit`}
                          render={({ field: input }) => (
                            <FormItem>
                              <FormLabel>Credits per unit</FormLabel>
                              <FormControl>
                                <Input
                                  {...input}
                                  inputMode="decimal"
                                  aria-label={`Component ${index + 1} price`}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <div className="flex items-center justify-between">
                          {saved?.sync_status && (
                            <Badge
                              variant={
                                saved.sync_status === "synced"
                                  ? "success"
                                  : "warning"
                              }
                            >
                              {saved.sync_status === "synced"
                                ? "Synced"
                                : saved.sync_status === "failed"
                                  ? "Failed"
                                  : "Pending"}
                            </Badge>
                          )}
                          <Button
                            type="button"
                            variant="outline"
                            onClick={() =>
                              setLane({
                                ...lane,
                                components: lane.components?.filter(
                                  (_, i) => i !== index,
                                ),
                              })
                            }
                          >
                            Remove component
                          </Button>
                        </div>
                      </div>
                    );
                  })}
                  <Button
                    type="button"
                    variant="outline"
                    disabled={
                      (lane.components?.length ?? 0) >=
                      suggestedMetrics.length - 1
                    }
                    onClick={() => {
                      const metric = suggestedMetrics.find(
                        (metric) =>
                          metric !== lane.metric &&
                          !lane.components?.some(
                            (component) => component.metric === metric,
                          ),
                      );
                      if (metric)
                        setLane({
                          ...lane,
                          components: [
                            ...(lane.components ?? []),
                            { metric, credits_per_unit: "0" },
                          ],
                        });
                    }}
                  >
                    Add component
                  </Button>
                  <p className="text-11 text-muted-foreground">
                    Prices support up to 12 decimal places. Each unit is charged
                    separately.
                  </p>
                </>
              )}
            </div>
          );
        })}
      </div>
      {(form.watch("byok_pricing") || form.watch("platform_key_pricing")) && (
        <p className="text-xs text-muted-foreground">
          Billing lanes supersede legacy platform billing below. A lane set to
          Free has no lane charge. Resale billing remains separate.
        </p>
      )}
    </section>
  );
}

type OperationPrice = {
  readonly operation: string;
  readonly credits_per_unit: string;
  readonly sync_status?: "pending" | "synced" | "failed";
};

function OperationPrices({
  operations,
  prices,
  saved,
  errorAt,
  onChange,
}: {
  readonly operations: readonly {
    readonly operation: string;
    readonly label: string;
  }[];
  readonly prices: readonly OperationPrice[];
  readonly saved: readonly OperationPrice[];
  /** Validation message for the price at this index of `prices`. */
  readonly errorAt: (index: number) => string | undefined;
  readonly onChange: (prices: OperationPrice[]) => void;
}) {
  if (!operations.length) return null;
  return (
    <div className="space-y-2 border-t border-border/50 pt-3">
      <div className="text-12 font-medium">Operation prices</div>
      <p className="text-11 text-muted-foreground">
        A synced operation price replaces the base Requests price. Leave an
        operation empty to use the base price.
      </p>
      {operations.map(({ operation, label }) => {
        const index = prices.findIndex((item) => item.operation === operation);
        const price = prices[index];
        const error = index >= 0 ? errorAt(index) : undefined;
        const status = saved.find(
          (item) =>
            item.operation === operation &&
            item.credits_per_unit === price?.credits_per_unit,
        )?.sync_status;
        return (
          <div
            key={operation}
            className="grid grid-cols-[1fr_8rem_4.5rem] items-center gap-2"
          >
            <span className="truncate text-12" title={operation}>
              {label}
            </span>
            <Input
              aria-label={`${label} operation price`}
              aria-invalid={Boolean(error)}
              inputMode="decimal"
              placeholder="Base price"
              value={price?.credits_per_unit ?? ""}
              onChange={(event) => {
                const value = event.target.value;
                const next = { operation, credits_per_unit: value };
                onChange(
                  !value
                    ? prices.filter((item) => item.operation !== operation)
                    : index >= 0
                      ? prices.map((item, at) => (at === index ? next : item))
                      : [...prices, next],
                );
              }}
            />
            {status ? (
              <Badge variant={status === "synced" ? "success" : "warning"}>
                {status === "synced"
                  ? "Synced"
                  : status === "failed"
                    ? "Failed"
                    : "Pending"}
              </Badge>
            ) : (
              <span />
            )}
            {error && (
              <p className="col-span-3 text-11 font-medium text-destructive">
                {error}
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}

import {
  forwardRef,
  useEffect,
  useMemo,
  useState,
  type InputHTMLAttributes,
  type KeyboardEvent,
} from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  AlertTriangle,
  Check,
  KeyRound,
  Plus,
  Settings2,
  ShieldCheck,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useDeletePlatformCredential,
  useDemotePlatformProvider,
  usePlatformOperations,
  usePlatformProviders,
  usePromotePlatformProvider,
  useSetPlatformCredential,
  useUpdatePlatformOperation,
} from "@/hooks/use-platform-ops";
import { ApiError } from "@/lib/api-client";
import {
  billingMetricSchema,
  platformCredentialWriteSchema,
  updatePlatformOperationSchema,
  type AdminPlatformOperation,
  type AdminPlatformProvider,
  type BillingMetric,
  type PlatformCredentialWrite,
  type UpdateAdminPlatformOperation,
} from "@/schemas/platform-ops";

const BILLING_METRICS = billingMetricSchema.options;
const EMPTY_OPERATIONS: AdminPlatformOperation[] = [];
const EMPTY_PROVIDERS: AdminPlatformProvider[] = [];

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof ApiError ? error.message : fallback;
}

type NumberInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "max" | "min" | "onChange" | "type" | "value"
> & {
  readonly value: number;
  readonly onChange: (value: number) => void;
  readonly min: number;
  readonly max: number;
};

const NumberInput = forwardRef<HTMLInputElement, NumberInputProps>(
  ({ value, onChange, min, max, ...props }, ref) => (
    <Input
      {...props}
      ref={ref}
      type="number"
      min={min}
      max={max}
      step={1}
      value={Number.isFinite(value) ? value : ""}
      onChange={(event) => onChange(event.target.valueAsNumber)}
    />
  ),
);
NumberInput.displayName = "NumberInput";

function StringListEditor({
  inputId,
  value,
  onChange,
  placeholder,
  itemLabel,
}: {
  readonly inputId: string;
  readonly value: readonly string[];
  readonly onChange: (value: string[]) => void;
  readonly placeholder: string;
  readonly itemLabel: string;
}) {
  const [draft, setDraft] = useState("");

  const addDraft = () => {
    const next = draft.trim();
    if (!next) return;
    if (!value.includes(next)) onChange([...value, next]);
    setDraft("");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== "Enter" && event.key !== ",") return;
    event.preventDefault();
    addDraft();
  };

  return (
    <div className="space-y-2">
      <div className="flex min-h-8 flex-wrap gap-1.5">
        {value.map((item) => (
          <span
            key={item}
            className="inline-flex h-7 max-w-full items-center gap-1 rounded-md border border-border bg-muted px-2 text-[11px] text-foreground"
          >
            <span className="truncate font-mono">{item}</span>
            <button
              type="button"
              className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-overlay-strong hover:text-foreground"
              onClick={() => onChange(value.filter((entry) => entry !== item))}
              aria-label={`Remove ${itemLabel} ${item}`}
              title={`Remove ${item}`}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          id={inputId}
          value={draft}
          placeholder={placeholder}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={addDraft}
        />
        <Button
          type="button"
          size="icon"
          variant="outline"
          onMouseDown={(event) => event.preventDefault()}
          onClick={addDraft}
          disabled={!draft.trim()}
          aria-label={`Add ${itemLabel}`}
          title={`Add ${itemLabel}`}
        >
          <Plus className="h-3 w-3" />
        </Button>
      </div>
    </div>
  );
}

function operationUpdate(
  operation: AdminPlatformOperation,
): UpdateAdminPlatformOperation {
  return updatePlatformOperationSchema.parse({
    enabled: operation.enabled,
    kind:
      operation.kind.type === "endpoint"
        ? {
            kind: "endpoint",
            method: operation.kind.method,
            path_template: operation.kind.path_template,
            name: operation.kind.name,
            description: operation.kind.description,
          }
        : {
            kind: "constrained",
            op: operation.kind.op,
            config: operation.kind.config,
          },
    limits: {
      ...operation.limits,
      per_user_per_day: operation.limits.per_user_per_day ?? 1,
    },
    billing: {
      metric: operation.pricing.metric,
      price_per_unit: operation.pricing.price_per_unit,
      secondary: operation.pricing.secondary
        ? {
            metric: operation.pricing.secondary.metric,
            price_per_unit: operation.pricing.secondary.price_per_unit,
          }
        : null,
      base_fee_per_call: operation.pricing.base_fee_per_call,
    },
  });
}

function normalizedOperationUpdate(
  update: UpdateAdminPlatformOperation,
): UpdateAdminPlatformOperation {
  if (
    update.kind.kind !== "constrained" ||
    update.kind.op !== "speak" ||
    update.kind.config.type !== "speak"
  ) {
    return update;
  }
  return {
    ...update,
    kind: {
      ...update.kind,
      config: {
        ...update.kind.config,
        max_calls_per_user_per_day: update.limits.per_user_per_day,
      },
    },
  };
}

function kindLabel(operation: AdminPlatformOperation): string {
  return operation.kind.type === "endpoint" ? "Endpoint" : "Constrained";
}

function metricLabel(metric: BillingMetric): string {
  return metric.replaceAll("_", " ");
}

function limitLabel(operation: AdminPlatformOperation): string {
  const daily = operation.limits.per_user_per_day
    ? `${String(operation.limits.per_user_per_day)}/owner/day`
    : "Daily cap missing";
  switch (operation.limits.per_request.type) {
    case "endpoint":
      return daily;
    case "speak":
      return `${String(operation.limits.per_request.max_chars)} chars/call, ${daily}`;
    case "call_and_say":
      return `${String(operation.limits.per_request.max_message_chars)} chars, ${String(operation.limits.per_request.max_duration_seconds)}s, ${daily}`;
    case "flight_search":
      return `${String(operation.limits.per_request.max_offers)} offers/call, ${daily}`;
  }
}

function BillingMetricSelect({
  value,
  onChange,
  disabled,
  label,
}: {
  readonly value: BillingMetric;
  readonly onChange: (value: BillingMetric) => void;
  readonly disabled?: boolean;
  readonly label: string;
}) {
  return (
    <Select
      value={value}
      onValueChange={(next) => onChange(next as BillingMetric)}
      disabled={disabled}
    >
      <SelectTrigger aria-label={label}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {BILLING_METRICS.map((metric) => (
          <SelectItem key={metric} value={metric}>
            {metricLabel(metric)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function EndpointFields({
  form,
}: {
  readonly form: ReturnType<typeof useAppForm<UpdateAdminPlatformOperation>>;
}) {
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-[112px_1fr] gap-3">
        <FormField
          control={form.control}
          name="kind.method"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Method</FormLabel>
              <FormControl>
                <Input
                  {...field}
                  value={typeof field.value === "string" ? field.value : ""}
                  className="font-mono uppercase"
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="kind.path_template"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Canonical Path</FormLabel>
              <FormControl>
                <Input
                  {...field}
                  value={typeof field.value === "string" ? field.value : ""}
                  className="font-mono"
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      </div>
      <FormField
        control={form.control}
        name="kind.name"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Operation Name</FormLabel>
            <FormControl>
              <Input
                {...field}
                value={typeof field.value === "string" ? field.value : ""}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormField
        control={form.control}
        name="kind.description"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Description</FormLabel>
            <FormControl>
              <textarea
                className="flex min-h-[80px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] text-foreground placeholder:text-text-tertiary focus-visible:border-white/[0.15] focus-visible:outline-none aria-invalid:border-destructive aria-invalid:focus-visible:border-destructive disabled:cursor-not-allowed disabled:opacity-50"
                value={typeof field.value === "string" ? field.value : ""}
                onBlur={field.onBlur}
                onChange={(event) =>
                  field.onChange(
                    event.target.value.trim() ? event.target.value : null,
                  )
                }
                name={field.name}
                ref={field.ref}
                rows={4}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
    </div>
  );
}

function ConstrainedFields({
  operation,
  form,
}: {
  readonly operation: AdminPlatformOperation;
  readonly form: ReturnType<typeof useAppForm<UpdateAdminPlatformOperation>>;
}) {
  if (operation.kind.type !== "constrained") return null;
  switch (operation.kind.op) {
    case "speak":
      return (
        <div className="space-y-4">
          <FormField
            control={form.control}
            name="kind.config.allowed_voice_ids"
            render={({ field }) => (
              <FormItem>
                <FormLabel htmlFor="platform-op-voice-id">
                  Allowed Voice IDs
                </FormLabel>
                <StringListEditor
                  inputId="platform-op-voice-id"
                  value={Array.isArray(field.value) ? field.value : []}
                  onChange={field.onChange}
                  placeholder="Voice ID"
                  itemLabel="voice ID"
                />
                <FormMessage />
              </FormItem>
            )}
          />
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="kind.config.model_id"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Model ID</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      value={typeof field.value === "string" ? field.value : ""}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="limits.per_request.max_chars"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Maximum Characters</FormLabel>
                  <FormControl>
                    <NumberInput
                      value={typeof field.value === "number" ? field.value : 0}
                      onChange={field.onChange}
                      min={1}
                      max={5_000}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
        </div>
      );
    case "call_and_say":
      return (
        <div className="space-y-4">
          <FormField
            control={form.control}
            name="kind.config.allowed_destination_prefixes"
            render={({ field }) => (
              <FormItem>
                <FormLabel htmlFor="platform-op-destination-prefix">
                  Allowed Destination Prefixes
                </FormLabel>
                <StringListEditor
                  inputId="platform-op-destination-prefix"
                  value={Array.isArray(field.value) ? field.value : []}
                  onChange={field.onChange}
                  placeholder="+65"
                  itemLabel="destination prefix"
                />
                <FormMessage />
              </FormItem>
            )}
          />
          <div className="grid gap-4 sm:grid-cols-2">
            {(["account_sid", "call_from", "voice"] as const).map((name) => (
              <FormField
                key={name}
                control={form.control}
                name={`kind.config.${name}`}
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      {name === "account_sid"
                        ? "Account SID"
                        : name === "call_from"
                          ? "Caller ID"
                          : "Voice"}
                    </FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        value={
                          typeof field.value === "string" ? field.value : ""
                        }
                        autoComplete="off"
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            ))}
            <FormField
              control={form.control}
              name="limits.per_request.max_message_chars"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Maximum Message Characters</FormLabel>
                  <FormControl>
                    <NumberInput
                      value={typeof field.value === "number" ? field.value : 0}
                      onChange={field.onChange}
                      min={1}
                      max={1_000}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="limits.per_request.max_duration_seconds"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Maximum Duration (seconds)</FormLabel>
                  <FormControl>
                    <NumberInput
                      value={typeof field.value === "number" ? field.value : 0}
                      onChange={field.onChange}
                      min={1}
                      max={3_600}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
        </div>
      );
    case "flight_search":
      return (
        <FormField
          control={form.control}
          name="limits.per_request.max_offers"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Maximum Offers</FormLabel>
              <FormControl>
                <NumberInput
                  value={typeof field.value === "number" ? field.value : 0}
                  onChange={field.onChange}
                  min={1}
                  max={50}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      );
  }
}

function BillingFields({
  operation,
  form,
}: {
  readonly operation: AdminPlatformOperation;
  readonly form: ReturnType<typeof useAppForm<UpdateAdminPlatformOperation>>;
}) {
  const secondary = form.watch("billing.secondary");
  const endpoint = operation.kind.type === "endpoint";
  return (
    <div className="space-y-4 border-t border-border/60 pt-5">
      <div>
        <h3 className="text-[13px] font-semibold">Pricing</h3>
        <p className="mt-1 text-[11px] text-muted-foreground">
          Lago code: {operation.pricing.lago_metric_code || "Pending sync"}
        </p>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <FormField
          control={form.control}
          name="billing.metric"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Primary Metric</FormLabel>
              <BillingMetricSelect
                value={field.value}
                onChange={field.onChange}
                disabled={!endpoint}
                label="Primary Metric"
              />
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="billing.price_per_unit"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Primary Price</FormLabel>
              <FormControl>
                <Input {...field} inputMode="decimal" className="font-mono" />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="billing.base_fee_per_call"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Base Fee Per Call</FormLabel>
              <FormControl>
                <Input
                  value={field.value ?? ""}
                  onBlur={field.onBlur}
                  onChange={(event) =>
                    field.onChange(event.target.value || null)
                  }
                  name={field.name}
                  ref={field.ref}
                  inputMode="decimal"
                  placeholder="No base fee"
                  className="font-mono"
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      </div>
      {endpoint && (
        <div className="space-y-4 rounded-md border border-border/60 bg-muted/20 p-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-[12px] font-medium">Secondary component</p>
              <p className="text-[11px] text-muted-foreground">
                Optional second measured quantity.
              </p>
            </div>
            <Switch
              checked={secondary !== null}
              onCheckedChange={(checked) =>
                form.setValue(
                  "billing.secondary",
                  checked
                    ? {
                        metric:
                          form.getValues("billing.metric") === "input_tokens"
                            ? "output_tokens"
                            : "input_tokens",
                        price_per_unit: "0.000001",
                      }
                    : null,
                )
              }
              aria-label="Enable secondary billing component"
            />
          </div>
          {secondary && (
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="billing.secondary.metric"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Secondary Metric</FormLabel>
                    <BillingMetricSelect
                      value={field.value}
                      onChange={field.onChange}
                      label="Secondary Metric"
                    />
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="billing.secondary.price_per_unit"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Secondary Price</FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        inputMode="decimal"
                        className="font-mono"
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
          )}
        </div>
      )}
      <div className="rounded-md border border-border/60 bg-muted/20 p-3 text-[11px]">
        <div className="flex items-center justify-between gap-3">
          <span className="text-muted-foreground">Billing sync</span>
          <SyncBadge operation={operation} />
        </div>
        {operation.pricing.sync_error && (
          <p className="mt-2 break-words text-destructive">
            {operation.pricing.sync_error}
          </p>
        )}
      </div>
    </div>
  );
}

function OperationDrawer({
  operation,
  onClose,
}: {
  readonly operation: AdminPlatformOperation | null;
  readonly onClose: () => void;
}) {
  const update = useUpdatePlatformOperation();
  const form = useAppForm<UpdateAdminPlatformOperation>({
    resolver: zodResolver(updatePlatformOperationSchema),
    defaultValues: operation ? operationUpdate(operation) : undefined,
  });

  useEffect(() => {
    if (operation) form.reset(operationUpdate(operation));
  }, [form, operation]);

  const onSubmit = async (data: UpdateAdminPlatformOperation) => {
    if (!operation) return;
    try {
      await update.mutateAsync({
        operationId: operation.operation_id,
        data: normalizedOperationUpdate(data),
      });
      toast.success("Platform operation saved");
      onClose();
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update platform operation"));
    }
  };

  return (
    <Sheet
      open={operation !== null}
      onOpenChange={(open) => {
        if (!open && !update.isPending) onClose();
      }}
    >
      <SheetContent
        className="flex w-full flex-col gap-0 overflow-hidden p-0 sm:max-w-xl"
        onPointerDownOutside={(event) => {
          if (update.isPending) event.preventDefault();
        }}
        onEscapeKeyDown={(event) => {
          if (update.isPending) event.preventDefault();
        }}
      >
        {operation && (
          <Form {...form}>
            <form
              className="flex min-h-0 flex-1 flex-col"
              onSubmit={form.handleSubmit(onSubmit)}
            >
              <SheetHeader className="border-b border-border/60 px-5 py-5 pr-12">
                <div className="flex items-center gap-2">
                  <SheetTitle>{operation.operation_name}</SheetTitle>
                  <Badge variant={operation.enabled ? "success" : "secondary"}>
                    {operation.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
                <SheetDescription className="font-mono text-[11px]">
                  {operation.operation_id}
                </SheetDescription>
              </SheetHeader>
              <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-5">
                <FormField
                  control={form.control}
                  name="enabled"
                  render={({ field }) => (
                    <FormItem className="flex items-center justify-between rounded-md border border-border/60 bg-muted/20 p-3">
                      <div>
                        <FormLabel>Enabled</FormLabel>
                        <p className="mt-0.5 text-[11px] text-muted-foreground">
                          Grants this exact operation row authority.
                        </p>
                      </div>
                      <FormControl>
                        <Switch
                          checked={field.value}
                          onCheckedChange={field.onChange}
                        />
                      </FormControl>
                    </FormItem>
                  )}
                />
                {operation.kind.type === "endpoint" ? (
                  <EndpointFields form={form} />
                ) : (
                  <ConstrainedFields operation={operation} form={form} />
                )}
                <FormField
                  control={form.control}
                  name="limits.per_user_per_day"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Daily Calls Per Owner</FormLabel>
                      <FormControl>
                        <NumberInput
                          value={field.value}
                          onChange={field.onChange}
                          min={1}
                          max={4_294_967_295}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <BillingFields operation={operation} form={form} />
                <FormSubmitErrors />
              </div>
              <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-5 py-4">
                <Button
                  type="button"
                  variant="outline"
                  onClick={onClose}
                  disabled={update.isPending}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!form.formState.isDirty || update.isPending}
                  isLoading={update.isPending}
                >
                  <Check className="h-3 w-3" />
                  Save changes
                </Button>
              </div>
            </form>
          </Form>
        )}
      </SheetContent>
    </Sheet>
  );
}

function ProviderDrawer({
  provider,
  onClose,
}: {
  readonly provider: AdminPlatformProvider | null;
  readonly onClose: () => void;
}) {
  const [termsAccepted, setTermsAccepted] = useState(false);
  const promote = usePromotePlatformProvider();
  const demote = useDemotePlatformProvider();
  const setCredential = useSetPlatformCredential();
  const deleteCredential = useDeletePlatformCredential();
  const form = useAppForm<PlatformCredentialWrite>({
    resolver: zodResolver(platformCredentialWriteSchema),
    defaultValues: { credential: "" },
  });
  const pending =
    promote.isPending ||
    demote.isPending ||
    setCredential.isPending ||
    deleteCredential.isPending;

  const run = async (action: () => Promise<unknown>, message: string) => {
    try {
      await action();
      toast.success(message);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update platform provider"));
    }
  };

  return (
    <Sheet
      open={provider !== null}
      onOpenChange={(open) => {
        if (!open && !pending) onClose();
      }}
    >
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        {provider && (
          <div className="space-y-6">
            <SheetHeader>
              <div className="flex items-center gap-2">
                <SheetTitle>{provider.catalog_service_name}</SheetTitle>
                <Badge variant={provider.eligible ? "success" : "destructive"}>
                  {provider.eligible ? "Eligible" : "Ineligible"}
                </Badge>
              </div>
              <SheetDescription className="font-mono text-[11px]">
                {provider.catalog_service_slug}
              </SheetDescription>
            </SheetHeader>

            {!provider.eligible && provider.eligibility_reason && (
              <div className="flex gap-2 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-[12px] text-destructive">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <p>{provider.eligibility_reason}</p>
              </div>
            )}

            <div className="space-y-3 border-b border-border/60 pb-6">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-[13px] font-semibold">Promotion</h3>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {String(provider.enabled_operation_count)} enabled
                    operations
                  </p>
                </div>
                <Badge variant={provider.promoted ? "success" : "secondary"}>
                  {provider.promoted ? "Promoted" : "Not promoted"}
                </Badge>
              </div>
              {!provider.promoted ? (
                <div className="space-y-3">
                  <label className="flex items-start gap-2 text-[12px] text-muted-foreground">
                    <Checkbox
                      checked={termsAccepted}
                      onCheckedChange={(checked) =>
                        setTermsAccepted(checked === true)
                      }
                      disabled={!provider.eligible || pending}
                      aria-label="Accept vendor terms"
                    />
                    <span>
                      I confirm the provider-specific vendor terms review is
                      complete.
                    </span>
                  </label>
                  <Button
                    variant="primary"
                    disabled={!provider.eligible || !termsAccepted || pending}
                    isLoading={promote.isPending}
                    onClick={() =>
                      void run(
                        () => promote.mutateAsync(provider.catalog_service_id),
                        "Provider promoted",
                      )
                    }
                  >
                    <ShieldCheck className="h-3 w-3" />
                    Promote provider
                  </Button>
                </div>
              ) : (
                <Button
                  variant="destructive"
                  disabled={pending}
                  isLoading={demote.isPending}
                  onClick={() =>
                    void run(
                      () => demote.mutateAsync(provider.catalog_service_id),
                      "Provider demoted",
                    )
                  }
                >
                  Demote provider
                </Button>
              )}
            </div>

            <div className="space-y-4">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-[13px] font-semibold">
                    Platform Credential
                  </h3>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {provider.credential.auth_method
                      ? `${provider.credential.auth_method} via ${provider.credential.auth_key_name ?? "provider default"}`
                      : "No credential metadata"}
                  </p>
                </div>
                <Badge
                  variant={
                    provider.credential.configured ? "success" : "secondary"
                  }
                >
                  {provider.credential.configured ? "Configured" : "Missing"}
                </Badge>
              </div>
              <Form {...form}>
                <form
                  className="space-y-3"
                  onSubmit={form.handleSubmit(async (data) => {
                    await run(
                      () =>
                        setCredential.mutateAsync({
                          providerId: provider.catalog_service_id,
                          data,
                        }),
                      provider.credential.configured
                        ? "Credential replaced"
                        : "Credential configured",
                    );
                    form.reset({ credential: "" });
                  })}
                >
                  <FormField
                    control={form.control}
                    name="credential"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>
                          {provider.credential.configured
                            ? "Replacement Credential"
                            : "Credential"}
                        </FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            type="password"
                            autoComplete="new-password"
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="submit"
                      variant="primary"
                      disabled={
                        !provider.promoted || !form.formState.isDirty || pending
                      }
                      isLoading={setCredential.isPending}
                    >
                      <KeyRound className="h-3 w-3" />
                      {provider.credential.configured ? "Replace" : "Configure"}
                    </Button>
                    {provider.credential.configured && (
                      <Button
                        type="button"
                        variant="destructive"
                        disabled={pending}
                        isLoading={deleteCredential.isPending}
                        onClick={() =>
                          void run(
                            () =>
                              deleteCredential.mutateAsync(
                                provider.catalog_service_id,
                              ),
                            "Credential deleted",
                          )
                        }
                      >
                        Delete credential
                      </Button>
                    )}
                  </div>
                </form>
              </Form>
            </div>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

function SyncBadge({
  operation,
}: {
  readonly operation: AdminPlatformOperation;
}) {
  const status = operation.pricing.sync_status;
  return (
    <Badge
      variant={
        status === "synced"
          ? "success"
          : status === "failed"
            ? "destructive"
            : "warning"
      }
      title={operation.pricing.sync_error ?? undefined}
    >
      {status}
    </Badge>
  );
}

function OperationCells({
  operation,
  providerName,
}: {
  readonly operation: AdminPlatformOperation;
  readonly providerName: string;
}) {
  return (
    <>
      <TableCell>
        <span className="text-[12px] text-muted-foreground">
          {providerName}
        </span>
      </TableCell>
      <TableCell>
        <p className="text-[12px] font-medium">{operation.operation_name}</p>
        {operation.kind.type === "endpoint" && (
          <p className="mt-0.5 max-w-[280px] truncate font-mono text-[10px] text-muted-foreground">
            {operation.kind.method} {operation.kind.path_template}
          </p>
        )}
      </TableCell>
      <TableCell>
        <Badge variant="secondary">{kindLabel(operation)}</Badge>
      </TableCell>
      <TableCell>
        <Badge variant={operation.enabled ? "success" : "secondary"}>
          {operation.enabled ? "Enabled" : "Disabled"}
        </Badge>
      </TableCell>
      <TableCell className="capitalize">
        <span className="text-[11px]">
          {metricLabel(operation.pricing.metric)}
        </span>
        {operation.pricing.secondary && (
          <span className="block text-[10px] text-muted-foreground">
            + {metricLabel(operation.pricing.secondary.metric)}
          </span>
        )}
      </TableCell>
      <TableCell className="max-w-[240px] text-[11px]">
        {operation.pricing.display}
      </TableCell>
      <TableCell className="max-w-[220px] text-[11px] text-muted-foreground">
        {limitLabel(operation)}
      </TableCell>
      <TableCell>
        <SyncBadge operation={operation} />
      </TableCell>
    </>
  );
}

export function AdminPlatformOpsPage() {
  const operationsQuery = usePlatformOperations();
  const providersQuery = usePlatformProviders();
  const [selectedOperationId, setSelectedOperationId] = useState<string | null>(
    null,
  );
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(
    null,
  );
  const operations = operationsQuery.data?.operations ?? EMPTY_OPERATIONS;
  const providers = providersQuery.data?.providers ?? EMPTY_PROVIDERS;
  const providerById = useMemo(
    () =>
      new Map(
        providers.map((provider) => [provider.catalog_service_id, provider]),
      ),
    [providers],
  );
  const grouped = useMemo(() => {
    const groups = new Map<string, AdminPlatformOperation[]>();
    for (const operation of operations) {
      const current = groups.get(operation.catalog_service_id) ?? [];
      current.push(operation);
      groups.set(operation.catalog_service_id, current);
    }
    for (const provider of providers) {
      if (!groups.has(provider.catalog_service_id)) {
        groups.set(provider.catalog_service_id, []);
      }
    }
    return [...groups.entries()].sort(([leftId], [rightId]) => {
      const left = providerById.get(leftId)?.catalog_service_name ?? leftId;
      const right = providerById.get(rightId)?.catalog_service_name ?? rightId;
      return left.localeCompare(right);
    });
  }, [operations, providerById, providers]);
  const selectedOperation =
    operations.find(
      (operation) => operation.operation_id === selectedOperationId,
    ) ?? null;
  const selectedProvider =
    providers.find(
      (provider) => provider.catalog_service_id === selectedProviderId,
    ) ?? null;
  const error = operationsQuery.error ?? providersQuery.error;
  const loading = operationsQuery.isLoading || providersQuery.isLoading;

  const openOperation = (operationId: string) => {
    setSelectedProviderId(null);
    setSelectedOperationId(operationId);
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Platform Operations"
        description="Manage provider eligibility, credentials, exact operation authority, limits, and pricing."
      />

      {error ? (
        <ErrorBanner
          message={errorMessage(error, "Failed to load platform operations")}
          onRetry={() => {
            void operationsQuery.refetch();
            void providersQuery.refetch();
          }}
        />
      ) : loading ? (
        <Skeleton className="h-[420px] w-full" />
      ) : grouped.length === 0 ? (
        <div className="rounded-xl border border-border/50 bg-card py-12 text-center">
          <p className="text-[12px] font-medium text-muted-foreground">
            No eligible platform providers found
          </p>
        </div>
      ) : (
        <>
          <div className="space-y-3 md:hidden">
            {grouped.map(([providerId, providerOperations]) => {
              const provider = providerById.get(providerId);
              const providerName =
                provider?.catalog_service_name ??
                providerOperations[0]?.provider_name ??
                "Deleted provider";
              return (
                <section key={providerId} className="space-y-2">
                  <div className="flex items-center justify-between gap-3 px-1">
                    <div className="min-w-0">
                      <p className="truncate text-[13px] font-semibold">
                        {providerName}
                      </p>
                      <p className="truncate font-mono text-[10px] text-muted-foreground">
                        {provider?.catalog_service_slug ??
                          providerOperations[0]?.provider_slug ??
                          providerId}
                      </p>
                    </div>
                    {provider && (
                      <Button
                        variant="outline"
                        size="icon"
                        onClick={() => setSelectedProviderId(providerId)}
                        aria-label={`Manage ${providerName}`}
                        title={`Manage ${providerName}`}
                      >
                        <Settings2 className="h-3 w-3" />
                      </Button>
                    )}
                  </div>
                  {providerOperations.length === 0 ? (
                    <div className="rounded-md border border-dashed border-border p-4 text-[11px] text-muted-foreground">
                      No operations configured
                    </div>
                  ) : (
                    providerOperations.map((operation) => (
                      <button
                        key={operation.operation_id}
                        type="button"
                        className="w-full rounded-md border border-border/50 bg-card p-4 text-left transition-colors hover:bg-white/[0.03]"
                        onClick={() => openOperation(operation.operation_id)}
                      >
                        <div className="flex items-start justify-between gap-2">
                          <div className="min-w-0">
                            <p className="truncate text-[13px] font-medium">
                              {operation.operation_name}
                            </p>
                            <p className="mt-0.5 text-[10px] capitalize text-muted-foreground">
                              {kindLabel(operation)} ·{" "}
                              {metricLabel(operation.pricing.metric)}
                            </p>
                          </div>
                          <Badge
                            variant={
                              operation.enabled ? "success" : "secondary"
                            }
                          >
                            {operation.enabled ? "Enabled" : "Disabled"}
                          </Badge>
                        </div>
                        <dl className="mt-3 grid gap-2 text-[11px]">
                          <div className="flex justify-between gap-3">
                            <dt className="text-muted-foreground">Price</dt>
                            <dd className="text-right">
                              {operation.pricing.display}
                            </dd>
                          </div>
                          <div className="flex justify-between gap-3">
                            <dt className="text-muted-foreground">Limits</dt>
                            <dd className="text-right">
                              {limitLabel(operation)}
                            </dd>
                          </div>
                          <div className="flex items-center justify-between gap-3">
                            <dt className="text-muted-foreground">
                              Billing sync
                            </dt>
                            <dd>
                              <SyncBadge operation={operation} />
                            </dd>
                          </div>
                        </dl>
                      </button>
                    ))
                  )}
                </section>
              );
            })}
          </div>

          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Provider</TableHead>
                  <TableHead>Operation</TableHead>
                  <TableHead>Kind</TableHead>
                  <TableHead>Enabled</TableHead>
                  <TableHead>Metric</TableHead>
                  <TableHead>Price</TableHead>
                  <TableHead>Limits</TableHead>
                  <TableHead>Billing sync</TableHead>
                </TableRow>
              </TableHeader>
              {grouped.map(([providerId, providerOperations]) => {
                const provider = providerById.get(providerId);
                const providerName =
                  provider?.catalog_service_name ??
                  providerOperations[0]?.provider_name ??
                  "Deleted provider";
                return (
                  <TableBody key={providerId}>
                    <TableRow className="bg-muted/30 hover:bg-muted/30">
                      <TableCell colSpan={8} className="py-2">
                        <div className="flex items-center justify-between gap-4">
                          <div className="flex min-w-0 items-center gap-2.5">
                            <span className="font-medium">{providerName}</span>
                            <span className="truncate font-mono text-[10px] text-muted-foreground">
                              {provider?.catalog_service_slug ??
                                providerOperations[0]?.provider_slug ??
                                providerId}
                            </span>
                            {provider && (
                              <>
                                <Badge
                                  variant={
                                    provider.promoted ? "success" : "secondary"
                                  }
                                >
                                  {provider.promoted
                                    ? "Promoted"
                                    : "Not promoted"}
                                </Badge>
                                <Badge
                                  variant={
                                    provider.credential.configured
                                      ? "success"
                                      : "secondary"
                                  }
                                >
                                  {provider.credential.configured
                                    ? "Credential set"
                                    : "No credential"}
                                </Badge>
                              </>
                            )}
                          </div>
                          {provider && (
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => setSelectedProviderId(providerId)}
                            >
                              <Settings2 className="h-3 w-3" />
                              Provider
                            </Button>
                          )}
                        </div>
                      </TableCell>
                    </TableRow>
                    {providerOperations.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={8}
                          className="py-6 text-center text-[11px] text-muted-foreground"
                        >
                          No operations configured
                        </TableCell>
                      </TableRow>
                    ) : (
                      providerOperations.map((operation) => (
                        <TableRow
                          key={operation.operation_id}
                          className="cursor-pointer"
                          tabIndex={0}
                          onClick={() => openOperation(operation.operation_id)}
                          onKeyDown={(event) => {
                            if (event.key === "Enter" || event.key === " ") {
                              event.preventDefault();
                              openOperation(operation.operation_id);
                            }
                          }}
                        >
                          <OperationCells
                            operation={operation}
                            providerName={providerName}
                          />
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                );
              })}
            </Table>
          </div>
        </>
      )}

      <OperationDrawer
        operation={selectedOperation}
        onClose={() => setSelectedOperationId(null)}
      />
      <ProviderDrawer
        key={selectedProvider?.catalog_service_id ?? "closed"}
        provider={selectedProvider}
        onClose={() => setSelectedProviderId(null)}
      />
    </div>
  );
}

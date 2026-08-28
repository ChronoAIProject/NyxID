import {
  forwardRef,
  useEffect,
  useState,
  type InputHTMLAttributes,
  type KeyboardEvent,
} from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  Check,
  ExternalLink,
  PhoneCall,
  Plane,
  Plus,
  Volume2,
  X,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
import { toast } from "sonner";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  usePlatformOperations,
  useUpdatePlatformOperation,
} from "@/hooks/use-platform-ops";
import { ApiError } from "@/lib/api-client";
import {
  callAndSayUpdateSchema,
  flightSearchUpdateSchema,
  speakUpdateSchema,
  type CallAndSayOperation,
  type CallAndSayUpdate,
  type FlightSearchOperation,
  type FlightSearchUpdate,
  type PlatformOperation,
  type SpeakOperation,
  type SpeakUpdate,
} from "@/schemas/platform-ops";

function updateErrorMessage(error: unknown): string {
  return error instanceof ApiError
    ? error.message
    : "Failed to update platform operation";
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
          <Plus className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function OperationHeader({
  icon: Icon,
  title,
  enabled,
  onEnabledChange,
}: {
  readonly icon: typeof Volume2;
  readonly title: string;
  readonly enabled: boolean;
  readonly onEnabledChange: (enabled: boolean) => void;
}) {
  return (
    <CardHeader className="border-b border-border/60">
      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border bg-muted text-muted-foreground">
            <Icon className="h-4 w-4" />
          </span>
          <CardTitle>{title}</CardTitle>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="text-[11px] text-muted-foreground">
            {enabled ? "Enabled" : "Disabled"}
          </span>
          <Switch
            checked={enabled}
            onCheckedChange={onEnabledChange}
            aria-label={`Enable ${title}`}
          />
        </div>
      </div>
    </CardHeader>
  );
}

function OperationPricing({
  operation,
}: {
  readonly operation: PlatformOperation;
}) {
  // Rendered by the backend so every metric formats identically here, in
  // /keys, and in MCP tool descriptions.
  const price = operation.pricing.display;

  return (
    <div className="flex min-h-9 items-center justify-between gap-3 rounded-md border border-border/60 bg-muted/30 px-3 py-2 text-xs">
      <div className="min-w-0">
        <span className="text-muted-foreground">Platform price</span>
        <span className="ml-2 font-medium text-foreground">{price}</span>
      </div>
      {operation.vendor_service_id ? (
        <Link
          to="/services/$serviceId/edit"
          params={{ serviceId: operation.vendor_service_id }}
          className="inline-flex shrink-0 items-center gap-1 text-foreground hover:underline"
        >
          Edit service
          <ExternalLink className="h-3 w-3" />
        </Link>
      ) : (
        <span className="shrink-0 text-muted-foreground">Vendor missing</span>
      )}
    </div>
  );
}

function SpeakOperationCard({
  operation,
}: {
  readonly operation: SpeakOperation;
}) {
  const update = useUpdatePlatformOperation();
  const form = useAppForm<SpeakUpdate>({
    resolver: zodResolver(speakUpdateSchema),
    defaultValues: {
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    },
  });

  useEffect(() => {
    form.reset({
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    });
  }, [form, operation]);

  const onSubmit = async (data: SpeakUpdate) => {
    try {
      const saved = await update.mutateAsync({ op: "speak", data });
      if (saved.op === "speak") {
        form.reset({
          enabled: saved.enabled,
          vendor_service_slug: saved.vendor_service_slug,
          config: saved.config,
        });
      }
      toast.success("Speech configuration saved");
    } catch (error) {
      toast.error(updateErrorMessage(error));
    }
  };

  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onSubmit)}>
        <Card className="h-full">
          <OperationHeader
            icon={Volume2}
            title="Speak"
            enabled={form.watch("enabled")}
            onEnabledChange={(enabled) => form.setValue("enabled", enabled)}
          />
          <CardContent className="space-y-4 pt-4">
            <FormField
              control={form.control}
              name="vendor_service_slug"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Vendor Service Slug</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <OperationPricing operation={operation} />
            <FormField
              control={form.control}
              name="config.allowed_voice_ids"
              render={({ field }) => (
                <FormItem>
                  <FormLabel htmlFor="platform-op-voice-id">
                    Allowed Voice IDs
                  </FormLabel>
                  <StringListEditor
                    inputId="platform-op-voice-id"
                    value={field.value}
                    onChange={field.onChange}
                    placeholder="Voice ID"
                    itemLabel="voice ID"
                  />
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
              <FormField
                control={form.control}
                name="config.max_chars"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Maximum Characters</FormLabel>
                    <FormControl>
                      <NumberInput
                        value={field.value}
                        onChange={field.onChange}
                        min={1}
                        max={5000}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.max_calls_per_user_per_day"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Daily Calls Per User</FormLabel>
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
              <FormField
                control={form.control}
                name="config.model_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Model ID</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <FormSubmitErrors />
          </CardContent>
          <CardFooter>
            <Button
              type="submit"
              variant="primary"
              disabled={!form.formState.isDirty || update.isPending}
              isLoading={update.isPending}
            >
              <Check className="h-4 w-4" />
              Save Speak
            </Button>
          </CardFooter>
        </Card>
      </form>
    </Form>
  );
}

function CallAndSayOperationCard({
  operation,
}: {
  readonly operation: CallAndSayOperation;
}) {
  const update = useUpdatePlatformOperation();
  const form = useAppForm<CallAndSayUpdate>({
    resolver: zodResolver(callAndSayUpdateSchema),
    defaultValues: {
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    },
  });

  useEffect(() => {
    form.reset({
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    });
  }, [form, operation]);

  const onSubmit = async (data: CallAndSayUpdate) => {
    try {
      const saved = await update.mutateAsync({ op: "call_and_say", data });
      if (saved.op === "call_and_say") {
        form.reset({
          enabled: saved.enabled,
          vendor_service_slug: saved.vendor_service_slug,
          config: saved.config,
        });
      }
      toast.success("Call and Say configuration saved");
    } catch (error) {
      toast.error(updateErrorMessage(error));
    }
  };

  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onSubmit)}>
        <Card className="h-full">
          <OperationHeader
            icon={PhoneCall}
            title="Call and Say"
            enabled={form.watch("enabled")}
            onEnabledChange={(enabled) => form.setValue("enabled", enabled)}
          />
          <CardContent className="space-y-4 pt-4">
            <FormField
              control={form.control}
              name="vendor_service_slug"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Vendor Service Slug</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <OperationPricing operation={operation} />
            <FormField
              control={form.control}
              name="config.allowed_destination_prefixes"
              render={({ field }) => (
                <FormItem>
                  <FormLabel htmlFor="platform-op-destination-prefix">
                    Allowed Destination Prefixes
                  </FormLabel>
                  <StringListEditor
                    inputId="platform-op-destination-prefix"
                    value={field.value}
                    onChange={field.onChange}
                    placeholder="+65"
                    itemLabel="destination prefix"
                  />
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
              <FormField
                control={form.control}
                name="config.account_sid"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Account SID</FormLabel>
                    <FormControl>
                      <Input {...field} autoComplete="off" />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.call_from"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Caller ID</FormLabel>
                    <FormControl>
                      <Input {...field} inputMode="tel" placeholder="+14155550123" />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.voice"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Voice</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.max_message_chars"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Maximum Message Characters</FormLabel>
                    <FormControl>
                      <NumberInput
                        value={field.value}
                        onChange={field.onChange}
                        min={1}
                        max={1000}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.max_duration_seconds"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Maximum Call Duration (seconds)</FormLabel>
                    <FormControl>
                      <NumberInput
                        value={field.value}
                        onChange={field.onChange}
                        min={1}
                        max={3_600}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.max_calls_per_user_per_day"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Daily Calls Per User</FormLabel>
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
            </div>
            <FormSubmitErrors />
          </CardContent>
          <CardFooter>
            <Button
              type="submit"
              variant="primary"
              disabled={!form.formState.isDirty || update.isPending}
              isLoading={update.isPending}
            >
              <Check className="h-4 w-4" />
              Save Call and Say
            </Button>
          </CardFooter>
        </Card>
      </form>
    </Form>
  );
}

function FlightSearchOperationCard({
  operation,
}: {
  readonly operation: FlightSearchOperation;
}) {
  const update = useUpdatePlatformOperation();
  const form = useAppForm<FlightSearchUpdate>({
    resolver: zodResolver(flightSearchUpdateSchema),
    defaultValues: {
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    },
  });

  useEffect(() => {
    form.reset({
      enabled: operation.enabled,
      vendor_service_slug: operation.vendor_service_slug,
      config: operation.config,
    });
  }, [form, operation]);

  const onSubmit = async (data: FlightSearchUpdate) => {
    try {
      const saved = await update.mutateAsync({ op: "flight_search", data });
      if (saved.op === "flight_search") {
        form.reset({
          enabled: saved.enabled,
          vendor_service_slug: saved.vendor_service_slug,
          config: saved.config,
        });
      }
      toast.success("Flight Search configuration saved");
    } catch (error) {
      toast.error(updateErrorMessage(error));
    }
  };

  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onSubmit)}>
        <Card className="h-full">
          <OperationHeader
            icon={Plane}
            title="Flight Search"
            enabled={form.watch("enabled")}
            onEnabledChange={(enabled) => form.setValue("enabled", enabled)}
          />
          <CardContent className="space-y-4 pt-4">
            <FormField
              control={form.control}
              name="vendor_service_slug"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Vendor Service Slug</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <OperationPricing operation={operation} />
            <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
              <FormField
                control={form.control}
                name="config.max_offers_cap"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Maximum Offers</FormLabel>
                    <FormControl>
                      <NumberInput
                        value={field.value}
                        onChange={field.onChange}
                        min={1}
                        max={50}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="config.max_searches_per_user_per_day"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Daily Searches Per User</FormLabel>
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
            </div>
            <FormSubmitErrors />
          </CardContent>
          <CardFooter>
            <Button
              type="submit"
              variant="primary"
              disabled={!form.formState.isDirty || update.isPending}
              isLoading={update.isPending}
            >
              <Check className="h-4 w-4" />
              Save Flight Search
            </Button>
          </CardFooter>
        </Card>
      </form>
    </Form>
  );
}

function OperationCard({
  operation,
}: {
  readonly operation: PlatformOperation;
}) {
  switch (operation.op) {
    case "speak":
      return <SpeakOperationCard operation={operation} />;
    case "call_and_say":
      return <CallAndSayOperationCard operation={operation} />;
    case "flight_search":
      return <FlightSearchOperationCard operation={operation} />;
  }
}

export function AdminPlatformOpsPage() {
  const {
    data,
    error,
    isLoading,
    refetch: refetchOperations,
  } = usePlatformOperations();
  return (
    <div className="space-y-6">
      <PageHeader
        title="Platform Operations"
        description="Configure NyxID-owned vendor operations and their server-enforced limits."
      />

      {error ? (
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : "Failed to load platform operations"
          }
          onRetry={() => void refetchOperations()}
        />
      ) : isLoading ? (
        <div className="grid gap-4 xl:grid-cols-2">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton
              key={`platform-operation-${String(index)}`}
              className="h-[420px] w-full"
            />
          ))}
        </div>
      ) : (
        <div className="grid items-start gap-4 xl:grid-cols-2">
          {data?.operations.map((operation) => (
            <OperationCard key={operation.op} operation={operation} />
          ))}
        </div>
      )}
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAllocateCredits } from "@/hooks/use-billing-credits";
import {
  allocationFormSchema,
  type AllocationForm,
} from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ServicePicker } from "./credit-pickers";
import { RecipientTargetFields } from "./recipient-targets";
import { ServiceLimitsEditor } from "./service-limits-editor";
import { rolloutWarningMessage } from "./credit-grant-visibility";

const defaults: AllocationForm = {
  amount: "",
  recurrence: "one_time",
  expiry: { kind: "end_of_period" },
  expires_at: "",
  target_kind: "selected_users",
  target_user_ids: [],
  target_org_ids: [],
  target_group_ids: [],
  all_services: true,
  service_refs: [],
  service_limits: [],
  reason: "",
};

export function AllocateDialog({
  open,
  onOpenChange,
  services,
  servicesError,
  servicesLoading,
  retryServices,
  canRecurring,
  initialRecurrence = "one_time",
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly services: readonly DownstreamService[];
  readonly servicesError: boolean;
  readonly servicesLoading: boolean;
  readonly retryServices: () => void;
  readonly canRecurring: boolean;
  readonly initialRecurrence?: AllocationForm["recurrence"];
}) {
  const [lookupBlocked, setLookupBlocked] = useState(false);
  const form = useAppForm<AllocationForm>({
    resolver: zodResolver(allocationFormSchema),
    defaultValues: defaults,
    mode: "onChange",
  });
  const mutation = useAllocateCredits();
  useEffect(() => {
    if (open) form.reset({ ...defaults, recurrence: initialRecurrence });
  }, [open, initialRecurrence, form]);
  const submission = useRef<{ payload: string; key: string } | null>(null);
  const inFlight = useRef(false);
  const value = form.watch();
  const needsServices = !value.all_services || value.service_limits.length > 0;
  const blocked =
    lookupBlocked || (needsServices && (servicesError || servicesLoading));
  const close = (next: boolean) => {
    if (inFlight.current || mutation.isPending) return;
    if (!next) {
      form.reset(defaults);
      submission.current = null;
      mutation.reset();
    }
    onOpenChange(next);
  };
  async function submit(data: AllocationForm) {
    if (
      inFlight.current ||
      mutation.isPending ||
      blocked ||
      !form.formState.isDirty ||
      (data.recurrence !== "one_time" && !canRecurring)
    )
      return;
    const payload = JSON.stringify(data);
    if (submission.current?.payload !== payload)
      submission.current = { payload, key: crypto.randomUUID() };
    inFlight.current = true;
    try {
      const response = await mutation.mutateAsync({
        form: data,
        requestId: submission.current.key,
      });
      if (response.kind === "one_time") {
        toast.success(
          `Allocated credits to ${String(response.result.created_count)} owners${response.result.pending_activation_count ? `; ${String(response.result.pending_activation_count)} pending activation` : ""}`,
        );
        const warning = rolloutWarningMessage(response.result.recipients);
        if (warning) toast.warning(warning);
      } else
        toast.success(
          "Recurring allocation created. The current period is issued on the next billing update.",
        );
      form.reset(defaults);
      submission.current = null;
      onOpenChange(false);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to allocate credits. Retry with the same settings to safely resume.",
      );
    } finally {
      inFlight.current = false;
    }
  }
  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Allocate credits</DialogTitle>
          <DialogDescription>
            Allocate one shared credit total per recipient. Issued credits are
            used before purchased credits.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)}>
            <DialogBody className="space-y-5">
              <fieldset disabled={mutation.isPending} className="space-y-5">
                <FormField
                  control={form.control}
                  name="amount"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Credits per recipient</FormLabel>
                      <FormControl>
                        <Input inputMode="decimal" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="recurrence"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Repeat allocation</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={field.onChange}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="one_time">One time</SelectItem>
                          {canRecurring ? (
                            <>
                              <SelectItem value="daily">Daily</SelectItem>
                              <SelectItem value="weekly">Weekly</SelectItem>
                              <SelectItem value="monthly">Monthly</SelectItem>
                            </>
                          ) : null}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <RecipientTargetFields
                  onLookupBlocked={setLookupBlocked}
                  description={
                    value.recurrence === "one_time"
                      ? "Recipients are captured when this allocation is submitted. Retrying preserves that snapshot."
                      : "Each UTC period captures eligible recipients. Later signups and organization joins wait until the next period. Each issuance gets a fresh total and fresh service limits."
                  }
                />
                {value.recurrence === "one_time" ? (
                  <FormField
                    control={form.control}
                    name="expires_at"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Expiry (optional)</FormLabel>
                        <FormControl>
                          <Input type="datetime-local" {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : (
                  <>
                    <FormField
                      control={form.control}
                      name="expiry.kind"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Expiry for each issuance</FormLabel>
                          <Select
                            value={field.value}
                            onValueChange={(kind) =>
                              form.setValue(
                                "expiry",
                                kind === "after_days"
                                  ? { kind, days: 30 }
                                  : {
                                      kind:
                                        kind === "never"
                                          ? "never"
                                          : "end_of_period",
                                    },
                              )
                            }
                          >
                            <FormControl>
                              <SelectTrigger>
                                <SelectValue />
                              </SelectTrigger>
                            </FormControl>
                            <SelectContent>
                              <SelectItem value="end_of_period">
                                End of period
                              </SelectItem>
                              <SelectItem value="after_days">
                                After a number of days
                              </SelectItem>
                              <SelectItem value="never">Never</SelectItem>
                            </SelectContent>
                          </Select>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                    {value.expiry.kind === "after_days" ? (
                      <FormField
                        control={form.control}
                        name="expiry.days"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Days until expiry</FormLabel>
                            <FormControl>
                              <Input
                                type="number"
                                min={1}
                                max={3650}
                                {...field}
                                value={
                                  Number.isFinite(field.value)
                                    ? field.value
                                    : ""
                                }
                                onChange={(e) =>
                                  field.onChange(e.target.valueAsNumber)
                                }
                              />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    ) : null}
                  </>
                )}
                <FormField
                  control={form.control}
                  name="all_services"
                  render={({ field }) => (
                    <FormItem className="flex items-center justify-between">
                      <FormLabel>All services eligible</FormLabel>
                      <FormControl>
                        <Switch
                          checked={field.value}
                          onCheckedChange={(checked) => {
                            field.onChange(checked);
                            if (checked) form.setValue("service_refs", []);
                          }}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                {needsServices && servicesError ? (
                  <ErrorBanner
                    message="Services could not be loaded. Retry to verify service restrictions."
                    onRetry={retryServices}
                  />
                ) : null}
                {needsServices && servicesLoading ? (
                  <p role="status">Loading services…</p>
                ) : null}
                {!value.all_services && !servicesError && !servicesLoading ? (
                  <FormField
                    control={form.control}
                    name="service_refs"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Eligible services</FormLabel>
                        <FormControl>
                          <ServicePicker
                            services={services}
                            selected={field.value}
                            onChange={field.onChange}
                            multiple
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}
                <ServiceLimitsEditor
                  services={
                    value.all_services
                      ? services
                      : services.filter((s) =>
                          value.service_refs.includes(s.id),
                        )
                  }
                  disabled={servicesError || servicesLoading}
                />
                {(servicesError || servicesLoading) && !needsServices ? (
                  <p className="text-12 text-muted-foreground">
                    Service limits need the service catalog.{" "}
                    <Button
                      variant="link"
                      type="button"
                      onClick={retryServices}
                    >
                      Retry services
                    </Button>
                  </p>
                ) : null}
                <FormField
                  control={form.control}
                  name="reason"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Reason (optional)</FormLabel>
                      <FormControl>
                        <Input {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <p className="text-12 text-muted-foreground">
                  The amount is shared across eligible services. Service
                  ceilings do not set aside funds.{" "}
                  {value.recurrence !== "one_time"
                    ? "Recurring changes apply to future issuances; existing allocations keep their original limits."
                    : "You can reclaim the unused part of this allocation, including while requests finish."}
                </p>
              </fieldset>
              {mutation.isError ? (
                <p role="alert" className="text-12 text-destructive">
                  {mutation.error.message} Retry with unchanged settings to
                  safely resume the same allocation.
                </p>
              ) : null}
            </DialogBody>
            <DialogFooter className="flex-wrap">
              <FormSubmitErrors />
              <Button
                type="button"
                variant="outline"
                disabled={mutation.isPending}
                onClick={() => close(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  !form.formState.isDirty || !form.formState.isValid || blocked
                }
                isLoading={mutation.isPending}
              >
                Allocate credits
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

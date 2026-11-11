import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label, FieldHelp } from "@/components/ui/label";
import { useRef, useState } from "react";
import { scheduleFormSchema } from "@/schemas/billing-credits";
import { ServiceLimitsEditor } from "./service-limits-editor";
import { ErrorBanner } from "@/components/shared/error-banner";
import type { UseFormReturn } from "react-hook-form";
import type { CreditSchedule, ScheduleForm } from "@/schemas/billing-credits";
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
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
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
import { ServicePicker } from "./credit-pickers";
import { RecipientTargetFields } from "./recipient-targets";

type ScheduleFormApi = UseFormReturn<ScheduleForm>;

export function ScheduleDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  editingSchedule,
  onSubmit,
  allocationPolicies = false,
  servicesError = false,
  servicesLoading = false,
  retryServices,
}: {
  readonly allocationPolicies?: boolean;
  readonly servicesError?: boolean;
  readonly servicesLoading?: boolean;
  readonly retryServices?: () => void;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: ScheduleFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly editingSchedule: CreditSchedule | null;
  readonly onSubmit: (value: ScheduleForm) => Promise<void>;
}) {
  useUiTranslation();
  const [lookupBlocked, setLookupBlocked] = useState(false);
  const submitting = useRef(false);
  const values = form.watch();
  const exactAmount = allocationPolicies || values.amount !== undefined;
  const valid = scheduleFormSchema.safeParse(values).success;
  const blocked =
    lookupBlocked ||
    ((servicesError || servicesLoading) &&
      (!values.all_services || Boolean(values.service_limits?.length)));
  const expiry = form.watch("expiry");
  const allServices = form.watch("all_services");

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!pending && !submitting.current) onOpenChange(next);
      }}
    >
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>
            {editingSchedule
              ? translateUi("Edit credit schedule")
              : translateUi("Create credit schedule")}
          </DialogTitle>
          <DialogDescription>{translateUi("Issue a fresh shared credit total on a UTC cycle. Each issuance has its own service ceilings. Changes affect future periods only.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(async (data) => {
              if (
                pending ||
                submitting.current ||
                blocked ||
                !form.formState.isDirty ||
                !valid
              )
                return;
              submitting.current = true;
              try {
                await onSubmit(data);
              } finally {
                submitting.current = false;
              }
            })}
          >
            <DialogBody className="-mx-1 px-1">
              <div className="space-y-5 pb-1">
                <div className="grid gap-4 sm:grid-cols-2">
                  {exactAmount ? (
                    <FormField
                      control={form.control}
                      name="amount"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{translateUi("Credits per recipient")}</FormLabel>
                          <FormControl>
                            <Input
                              inputMode="decimal"
                              {...field}
                              value={field.value ?? ""}
                              disabled={!allocationPolicies}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  ) : (
                    <>
                      <FormField
                        control={form.control}
                        name="amount_credits"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>{translateUi("Credits per recipient")}</FormLabel>
                            <FormControl>
                              <Input
                                type="number"
                                min={1}
                                max={1_000_000}
                                {...field}
                                value={
                                  Number.isFinite(field.value)
                                    ? field.value
                                    : ""
                                }
                                onChange={(event) =>
                                  field.onChange(event.target.valueAsNumber)
                                }
                              />
                            </FormControl>
                            <FormDescription className="text-13">{translateUi("Issued as a separate credit allocation each period.")}</FormDescription>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    </>
                  )}
                  <FormField
                    control={form.control}
                    name="recurrence"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Recurrence")}</FormLabel>
                        <Select
                          value={field.value}
                          disabled={editingSchedule !== null}
                          onValueChange={field.onChange}
                        >
                          <FormControl>
                            <SelectTrigger>
                              <SelectValue />
                            </SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            <SelectItem value="daily">{translateUi("Daily")}</SelectItem>
                            <SelectItem value="weekly">{translateUi("Weekly")}</SelectItem>
                            <SelectItem value="monthly">{translateUi("Monthly")}</SelectItem>
                          </SelectContent>
                        </Select>
                        {editingSchedule ? (
                          <FormDescription className="text-13">{translateUi("Pause this schedule and create another to change its recurrence.")}</FormDescription>
                        ) : null}
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>

                <FormField
                  control={form.control}
                  name="expiry"
                  render={({ field }) => (
                    <FormItem>
                      <fieldset className="space-y-2">
                        <Label asChild><legend className="text-muted-foreground">{translateUi("Expiry policy")}</legend></Label>
                        <div className="grid gap-2 sm:grid-cols-3">
                          <ExpiryOption
                            label={translateUi("End of each period")}
                            description={translateUi("No rollover")}
                            checked={field.value.kind === "end_of_period"}
                            onChange={() =>
                              field.onChange({ kind: "end_of_period" })
                            }
                          />
                          <ExpiryOption
                            label={translateUi("After a fixed number of days")}
                            description={translateUi("From disbursement")}
                            checked={field.value.kind === "after_days"}
                            onChange={() =>
                              field.onChange({ kind: "after_days", days: 30 })
                            }
                          />
                          <ExpiryOption
                            label={translateUi("Never")}
                            description={translateUi("Credits do not expire")}
                            checked={field.value.kind === "never"}
                            onChange={() => field.onChange({ kind: "never" })}
                          />
                        </div>
                      </fieldset>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                {expiry.kind === "after_days" ? (
                  <FormField
                    control={form.control}
                    name="expiry.days"
                    render={({ field }) => (
                      <FormItem className="max-w-52">
                        <FormLabel>{translateUi("Days until expiry")}</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={3_650}
                            {...field}
                            value={
                              Number.isFinite(field.value) ? field.value : ""
                            }
                            onChange={(event) => {
                              field.onChange(event.target.valueAsNumber);
                              void form.trigger("expiry.days");
                            }}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}

                <RecipientTargetFields
                  onLookupBlocked={setLookupBlocked}
                  description={translateUi("Recipient policy is captured when each period opens. Later signups and organization joins wait until the next period.")}
                />

                <FormField
                  control={form.control}
                  name="all_services"
                  render={({ field }) => (
                    <FormItem className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
                      <div>
                        <FormLabel>{translateUi("All services")}</FormLabel>
                        <p className="text-13 text-muted-foreground">{translateUi("Allow each period's credits to fund any service.")}</p>
                      </div>
                      <FormControl>
                        <Switch
                          checked={field.value}
                          onCheckedChange={(checked) => {
                            field.onChange(checked);
                            if (checked) form.setValue("service_refs", []);
                          }}
                        />
                      </FormControl>
                    </FormItem>
                  )}
                />
                {servicesError ? (
                  <ErrorBanner
                    message={translateUi("Services could not be loaded. Retry to verify restrictions.")}
                    onRetry={retryServices}
                  />
                ) : null}
                {servicesLoading &&
                (!allServices || Boolean(values.service_limits?.length)) ? (
                  <p role="status">{translateUi("Loading services…")}</p>
                ) : null}
                {!allServices && !servicesError && !servicesLoading ? (
                  <FormField
                    control={form.control}
                    name="service_refs"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Service scope")}</FormLabel>
                        <FormControl>
                          <ServicePicker
                            services={services}
                            selected={field.value}
                            onChange={field.onChange}
                            multiple
                          />
                        </FormControl>
                        <FormDescription className="text-13">{translateUi("Service metrics do not change the scheduled credit amount.")}</FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}

                {allocationPolicies ||
                Boolean(editingSchedule?.service_limits?.length) ? (
                  <ServiceLimitsEditor
                    services={
                      allServices
                        ? services
                        : services.filter((service) =>
                            values.service_refs.includes(service.id),
                          )
                    }
                    disabled={
                      !allocationPolicies ||
                      pending ||
                      servicesError ||
                      servicesLoading
                    }
                  />
                ) : null}
                <FormField
                  control={form.control}
                  name="reason"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Reason / note")}</FormLabel>
                      <FormControl>
                        <textarea
                          rows={3}
                          className="w-full resize-y rounded-lg border border-input bg-transparent px-3 py-2 text-14 outline-none focus:border-input-focus"
                          placeholder={translateUi("Why these recurring credits are issued")}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
            </DialogBody>
            <DialogFooter className="pt-4 md:pt-4">
              <Button
                type="button"
                variant="ghost"
                disabled={pending}
                onClick={() => {
                  if (!pending && !submitting.current) onOpenChange(false);
                }}
              >{translateUi("Cancel")}</Button>
              <FormSubmitErrors />
              <Button
                type="submit"
                variant="primary"
                isLoading={pending}
                disabled={!form.formState.isDirty || !valid || blocked}
              >
                {editingSchedule ? translateUi("Save changes") : translateUi("Create schedule")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function ExpiryOption({
  label,
  description,
  checked,
  onChange,
}: {
  readonly label: string;
  readonly description: string;
  readonly checked: boolean;
  readonly onChange: () => void;
}) {
  useUiTranslation();
  return (
    <Label className="flex cursor-pointer gap-2 rounded-lg border border-border px-3 py-2">
      <input
        type="radio"
        name="schedule-expiry-policy"
        aria-label={label}
        className="mt-0.5 h-3.5 w-3.5 accent-primary"
        checked={checked}
        onChange={onChange}
      />
      <span>
        <span className="block">{label}</span>
        <FieldHelp className="block text-muted-foreground">
          {description}
        </FieldHelp>
      </span>
    </Label>
  );
}

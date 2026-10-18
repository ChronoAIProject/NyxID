import { useRef, useState } from "react";
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
import { ServiceLimitsEditor } from "./service-limits-editor";
import { ServicePicker, UserPicker } from "./credit-pickers";

type ScheduleFormApi = UseFormReturn<ScheduleForm>;

export function ScheduleDialog({
  open,
  onOpenChange,
  form,
  services,
  servicesLoading,
  servicesError,
  onRetryServices,
  pending,
  editingSchedule,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: ScheduleFormApi;
  readonly services: readonly DownstreamService[];
  readonly servicesLoading: boolean;
  readonly servicesError: boolean;
  readonly onRetryServices: () => void;
  readonly pending: boolean;
  readonly editingSchedule: CreditSchedule;
  readonly onSubmit: (value: ScheduleForm) => Promise<void>;
}) {
  const expiry = form.watch("expiry");
  const targetKind = form.watch("target_kind");
  const allServices = form.watch("all_services");
  const [ownersDiscoveryFailed, setOwnersDiscoveryFailed] = useState(false);
  const [ownersLoading, setOwnersLoading] = useState(false);
  const submitting = useRef(false);
  const targetIds = form.watch("target_user_ids");
  const amount = form.watch("amount_credits");
  const serviceRefs = form.watch("service_refs");
  const rules = form.watch("service_limits") ?? [];
  const needsServiceDiscovery = !allServices || rules.length > 0;
  const discoveryFailed =
    (targetKind === "selected_users" && ownersDiscoveryFailed) ||
    (needsServiceDiscovery && servicesError);
  const discoveryBlocked =
    discoveryFailed ||
    (targetKind === "selected_users" && ownersLoading) ||
    (needsServiceDiscovery && servicesLoading);
  const requiredFieldsPresent =
    Number.isFinite(amount) &&
    amount >= 1 &&
    rules.every(
      (rule) =>
        Number.isInteger(rule.amount_credits) &&
        rule.amount_credits >= 1 &&
        rule.service_refs.length > 0,
    ) &&
    (targetKind === "all_users" || targetIds.length > 0) &&
    (allServices || serviceRefs.length > 0) &&
    (expiry.kind !== "after_days" || Number.isFinite(expiry.days));
  const canSubmit =
    form.formState.isDirty &&
    requiredFieldsPresent &&
    !discoveryBlocked &&
    !pending;

  function handleOpenChange(nextOpen: boolean) {
    if (!pending && !submitting.current) onOpenChange(nextOpen);
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        scrollMode="body"
        className="overflow-clip md:max-w-2xl md:overflow-clip [&>div]:overflow-clip"
      >
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>Edit recurring credits</DialogTitle>
          <DialogDescription>
            Set the amount, recipients, expiry, and service limits for future
            allocations.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(async (value) => {
              if (!canSubmit || submitting.current) return;
              submitting.current = true;
              try {
                await onSubmit(value);
              } finally {
                submitting.current = false;
              }
            })}
          >
            <DialogBody className="-mx-1 px-1">
              <div className="space-y-5 pb-1">
                <div
                  className="rounded-lg bg-muted/40 p-3 text-[11px] text-muted-foreground [overflow-wrap:anywhere]"
                  aria-live="polite"
                >
                  <p className="font-medium text-foreground">
                    Shared total:{" "}
                    {Number.isFinite(amount)
                      ? new Intl.NumberFormat().format(amount)
                      : "—"}{" "}
                    credits per owner
                  </p>
                  {targetKind === "selected_users" &&
                  targetIds.length > 0 &&
                  Number.isFinite(amount) ? (
                    <p>
                      {new Intl.NumberFormat().format(amount)} ×{" "}
                      {targetIds.length} owners ={" "}
                      {new Intl.NumberFormat().format(
                        amount * targetIds.length,
                      )}{" "}
                      credits issued across selected owners.
                    </p>
                  ) : null}
                  {rules.map((rule, index) => (
                    <p key={index}>
                      {rule.label || `Limit ${String(index + 1)}`}:{" "}
                      {rule.service_refs
                        .map(
                          (ref) =>
                            services.find((service) => service.id === ref)
                              ?.name ?? ref,
                        )
                        .join(", ") || "Choose services"}{" "}
                      · up to{" "}
                      {Number.isFinite(rule.amount_credits)
                        ? new Intl.NumberFormat().format(rule.amount_credits)
                        : "—"}{" "}
                      credits from each allocation.
                    </p>
                  ))}
                  {rules.length ? (
                    <p>Limits share this total; no credits are set aside.</p>
                  ) : null}
                </div>
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="amount_credits"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Credits per owner</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={1_000_000}
                            {...field}
                            value={
                              Number.isFinite(field.value) ? field.value : ""
                            }
                            onChange={(event) =>
                              field.onChange(event.target.valueAsNumber)
                            }
                          />
                        </FormControl>
                        <FormDescription className="text-[11px]">
                          Allocates a fresh shared total for each UTC period.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name="recurrence"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Recurrence</FormLabel>
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
                            <SelectItem value="daily">Daily</SelectItem>
                            <SelectItem value="weekly">Weekly</SelectItem>
                            <SelectItem value="monthly">Monthly</SelectItem>
                          </SelectContent>
                        </Select>
                        <FormDescription className="text-[11px]">
                          Pause this policy and add another one to change its
                          recurrence.
                        </FormDescription>
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
                        <legend className="text-xs font-medium text-muted-foreground">
                          Expiry policy
                        </legend>
                        <div className="grid gap-2 sm:grid-cols-3">
                          <ExpiryOption
                            label="End of each period"
                            description="No rollover"
                            checked={field.value.kind === "end_of_period"}
                            onChange={() =>
                              field.onChange({ kind: "end_of_period" })
                            }
                          />
                          <ExpiryOption
                            label="After a fixed number of days"
                            description="From issuance"
                            checked={field.value.kind === "after_days"}
                            onChange={() =>
                              field.onChange({ kind: "after_days", days: 30 })
                            }
                          />
                          <ExpiryOption
                            label="Never"
                            description="Credits do not expire"
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
                        <FormLabel>Days until expiry</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={3_650}
                            {...field}
                            value={
                              Number.isFinite(field.value) ? field.value : ""
                            }
                            onChange={(event) =>
                              field.onChange(event.target.valueAsNumber)
                            }
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}

                <TargetFields
                  form={form}
                  targetKind={targetKind}
                  onOwnersError={setOwnersDiscoveryFailed}
                  onOwnersLoading={setOwnersLoading}
                />

                <FormField
                  control={form.control}
                  name="all_services"
                  render={({ field }) => (
                    <FormItem className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
                      <div>
                        <FormLabel>All services</FormLabel>
                        <p className="text-[11px] text-muted-foreground">
                          Allow each period&apos;s credits to fund any service.
                        </p>
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
                {!allServices ? (
                  <FormField
                    control={form.control}
                    name="service_refs"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Service scope</FormLabel>
                        <FormControl>
                          <ServicePicker
                            services={services}
                            selected={field.value}
                            onChange={field.onChange}
                            multiple
                            loading={servicesLoading}
                            error={servicesError}
                            onRetry={onRetryServices}
                          />
                        </FormControl>
                        <FormDescription className="text-[11px]">
                          Service metrics do not change the scheduled credit
                          amount.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}

                <ServiceLimitsEditor
                  rules={rules}
                  onChange={(value) => form.setValue("service_limits", value)}
                  errors={form.formState.errors.service_limits}
                  services={services}
                  loading={servicesLoading}
                  error={servicesError}
                  onRetry={onRetryServices}
                  disabled={pending || form.formState.isSubmitting}
                />
                <p
                  className="text-[11px] text-muted-foreground"
                  aria-live="polite"
                >
                  {Number.isFinite(amount)
                    ? new Intl.NumberFormat().format(amount)
                    : "—"}{" "}
                  credits per owner.{" "}
                  {targetKind === "all_users"
                    ? "Every eligible owner"
                    : `${String(targetIds.length)} selected owner${targetIds.length === 1 ? "" : "s"}`}{" "}
                  receives this amount; it is not split between recipients.
                  {rules.length
                    ? ` ${String(rules.length)} service limit${rules.length === 1 ? " shares" : "s share"} each allocation’s total.`
                    : " Eligible services share the total without additional limits."}{" "}
                  Each future allocation receives fresh limits and usage
                  counters. Changes apply to future issuance only.
                </p>
                <FormField
                  control={form.control}
                  name="reason"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Reason / note</FormLabel>
                      <FormControl>
                        <textarea
                          rows={3}
                          className="w-full resize-y rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] outline-none focus:border-white/15"
                          placeholder="Why these recurring credits are issued"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
            </DialogBody>
            <DialogFooter className="flex-wrap pt-4 md:pt-4">
              {discoveryBlocked || form.formState.submitCount > 0 ? (
                <div className="basis-full text-xs">
                  <FormSubmitErrors />
                  {discoveryBlocked ? (
                    <p
                      role={discoveryFailed ? "alert" : "status"}
                      className={
                        discoveryFailed
                          ? "text-destructive"
                          : "text-muted-foreground"
                      }
                    >
                      {discoveryFailed
                        ? "Required owners or services could not be loaded. Use Retry in the picker before submitting."
                        : "Loading owners or services. Please wait before submitting."}
                    </p>
                  ) : null}
                </div>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                disabled={pending || form.formState.isSubmitting}
                onClick={() => handleOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={!canSubmit || form.formState.isSubmitting}
                isLoading={pending || form.formState.isSubmitting}
              >
                Save changes
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
  return (
    <label className="flex cursor-pointer gap-2 rounded-lg border border-border px-3 py-2">
      <input
        type="radio"
        name="schedule-expiry-policy"
        aria-label={label}
        className="mt-0.5 h-3.5 w-3.5 accent-primary"
        checked={checked}
        onChange={onChange}
      />
      <span>
        <span className="block text-[12px] font-medium">{label}</span>
        <span className="block text-[11px] text-muted-foreground">
          {description}
        </span>
      </span>
    </label>
  );
}

function TargetFields({
  form,
  targetKind,
  onOwnersError,
  onOwnersLoading,
}: {
  readonly form: ScheduleFormApi;
  readonly onOwnersError: (failed: boolean) => void;
  readonly onOwnersLoading: (loading: boolean) => void;
  readonly targetKind: ScheduleForm["target_kind"];
}) {
  return (
    <>
      <FormField
        control={form.control}
        name="target_kind"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Recipients</FormLabel>
            <Select
              value={field.value}
              onValueChange={(value) => {
                field.onChange(value);
                if (value === "all_users") {
                  form.setValue("target_user_ids", []);
                }
              }}
            >
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="all_users">All billing owners</SelectItem>
                <SelectItem value="selected_users">Selected owners</SelectItem>
              </SelectContent>
            </Select>
            <FormDescription className="text-[11px]">
              All-owner membership is captured when each period opens.
            </FormDescription>
          </FormItem>
        )}
      />
      {targetKind === "selected_users" ? (
        <FormField
          control={form.control}
          name="target_user_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Owners</FormLabel>
              <FormControl>
                <UserPicker
                  selected={field.value}
                  onChange={field.onChange}
                  onErrorStateChange={onOwnersError}
                  onLoadingStateChange={onOwnersLoading}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}

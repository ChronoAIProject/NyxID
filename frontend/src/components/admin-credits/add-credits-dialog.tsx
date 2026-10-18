import { useRef, useState } from "react";
import type { UseFormReturn } from "react-hook-form";
import type { AddCreditsForm } from "@/schemas/billing-credits";
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

export function AddCreditsDialog({
  open,
  onOpenChange,
  form,
  services,
  servicesLoading,
  servicesError,
  onRetryServices,
  pending,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: UseFormReturn<AddCreditsForm>;
  readonly services: readonly DownstreamService[];
  readonly servicesLoading: boolean;
  readonly servicesError: boolean;
  readonly onRetryServices: () => void;
  readonly pending: boolean;
  readonly onSubmit: (value: AddCreditsForm) => Promise<void>;
}) {
  const targetKind = form.watch("target_kind");
  const allServices = form.watch("all_services");
  const recurrence = form.watch("recurrence");
  const expiryKind = form.watch("expiry.kind");
  const [ownersDiscoveryFailed, setOwnersDiscoveryFailed] = useState(false);
  const [ownersLoading, setOwnersLoading] = useState(false);
  const submitting = useRef(false);
  const amount = form.watch("amount_credits");
  const targetIds = form.watch("target_user_ids");
  const serviceRefs = form.watch("service_refs");
  const rules = form.watch("service_limits") ?? [];
  const needsServiceDiscovery = !allServices || rules.length > 0;
  const expiryDays = form.watch("expiry.days");
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
    (recurrence === "one_time" ||
      expiryKind !== "after_days" ||
      Number.isFinite(expiryDays));
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
          <DialogTitle>Allocate credits</DialogTitle>
          <DialogDescription>
            Allocate credits for one or more billing owners. Choose a single
            issuance or a recurring credit policy in this same flow.
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
                          Each selected recipient receives this amount.
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
                        <FormLabel>Issuance</FormLabel>
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
                            <SelectItem value="daily">
                              Recurring · daily
                            </SelectItem>
                            <SelectItem value="weekly">
                              Recurring · weekly
                            </SelectItem>
                            <SelectItem value="monthly">
                              Recurring · monthly
                            </SelectItem>
                          </SelectContent>
                        </Select>
                        <FormDescription className="text-[11px]">
                          A fresh allocation is issued at the start of each UTC
                          period.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>

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
                          <SelectItem value="all_users">
                            All billing owners
                          </SelectItem>
                          <SelectItem value="selected_users">
                            Selected owners
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <FormDescription className="text-[11px]">
                        All billing owners are resolved when a one-time issuance
                        is submitted or when each recurring UTC period starts.
                      </FormDescription>
                      <FormMessage />
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
                            onErrorStateChange={setOwnersDiscoveryFailed}
                            onLoadingStateChange={setOwnersLoading}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}

                {recurrence === "one_time" ? (
                  <FormField
                    control={form.control}
                    name="expires_at"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Expiry (optional)</FormLabel>
                        <FormControl>
                          <Input type="datetime-local" {...field} />
                        </FormControl>
                        <FormDescription className="text-[11px]">
                          Leave empty for credits that do not expire.
                        </FormDescription>
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
                          <FormLabel>Recurring credit expiry</FormLabel>
                          <Select
                            value={field.value}
                            onValueChange={(value) => {
                              if (value === "after_days") {
                                form.setValue("expiry", {
                                  kind: "after_days",
                                  days: 30,
                                });
                              } else if (value === "never") {
                                form.setValue("expiry", { kind: "never" });
                              } else {
                                form.setValue("expiry", {
                                  kind: "end_of_period",
                                });
                              }
                            }}
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
                    {expiryKind === "after_days" ? (
                      <FormField
                        control={form.control}
                        name="expiry.days"
                        render={({ field }) => (
                          <FormItem className="mt-2 max-w-52">
                            <FormLabel>Days until expiry</FormLabel>
                            <FormControl>
                              <Input
                                type="number"
                                min={1}
                                max={3_650}
                                value={
                                  typeof field.value === "number" &&
                                  Number.isFinite(field.value)
                                    ? field.value
                                    : ""
                                }
                                onChange={(event) =>
                                  field.onChange(event.target.valueAsNumber)
                                }
                                aria-label="Expiry days"
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
                    <FormItem className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
                      <div>
                        <FormLabel>All services</FormLabel>
                        <p className="text-[11px] text-muted-foreground">
                          Leave enabled unless these credits should be limited
                          to selected services.
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
                        <FormLabel>Eligible services</FormLabel>
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
                    : " Eligible services share the total without additional limits."}
                  {recurrence !== "one_time"
                    ? " Each recurring allocation receives fresh limits and usage counters."
                    : ""}
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
                          placeholder="Why these credits are being issued"
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
                <div className="basis-full text-xs md:mr-auto md:max-w-sm md:basis-auto">
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
                Allocate credits
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

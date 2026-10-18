import { useRef, useState } from "react";
import type { UseFormReturn } from "react-hook-form";
import {
  billingMetricLabel,
  formatAllowancePreview,
  resolveServiceBillingMetric,
} from "@/lib/billing-units";
import type { AllowanceForm, UsageAllowance } from "@/schemas/billing-credits";
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
import { ServicePicker, UserPicker } from "./credit-pickers";

type AllowanceFormApi = UseFormReturn<AllowanceForm>;

export function AllowanceDialog({
  open,
  onOpenChange,
  form,
  services,
  servicesLoading,
  servicesError,
  onRetryServices,
  pending,
  editingAllowance,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: AllowanceFormApi;
  readonly services: readonly DownstreamService[];
  readonly servicesLoading: boolean;
  readonly servicesError: boolean;
  readonly onRetryServices: () => void;
  readonly pending: boolean;
  readonly editingAllowance: UsageAllowance | null;
  readonly onSubmit: (value: AllowanceForm) => Promise<void>;
}) {
  const targetKind = form.watch("target_kind");
  const serviceRef = form.watch("service_ref");
  const quantity = form.watch("quantity");
  const recurrence = form.watch("recurrence");
  const [ownersDiscoveryFailed, setOwnersDiscoveryFailed] = useState(false);
  const [ownersLoading, setOwnersLoading] = useState(false);
  const submitting = useRef(false);
  const targetIds = form.watch("target_user_ids");
  const discoveryFailed =
    (targetKind === "selected_users" && ownersDiscoveryFailed) || servicesError;
  const discoveryBlocked =
    discoveryFailed ||
    (targetKind === "selected_users" && ownersLoading) ||
    servicesLoading;
  const requiredFieldsPresent =
    serviceRef.trim().length > 0 &&
    Number.isFinite(quantity) &&
    (targetKind === "all_users" || targetIds.length > 0);
  const canSubmit =
    form.formState.isDirty &&
    requiredFieldsPresent &&
    !discoveryBlocked &&
    !pending;

  const selectedService = services.find(
    (service) => service.id === serviceRef || service.slug === serviceRef,
  );
  const metric = selectedService
    ? resolveServiceBillingMetric(selectedService)
    : editingAllowance?.service_id === serviceRef
      ? editingAllowance.metric
      : null;
  const preview = metric
    ? formatAllowancePreview(quantity, metric, recurrence)
    : null;

  function handleOpenChange(nextOpen: boolean) {
    if (!pending && !submitting.current) onOpenChange(nextOpen);
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>
            {editingAllowance ? "Edit included usage" : "Add included usage"}
          </DialogTitle>
          <DialogDescription>
            Add included service usage before wallet credits are charged.
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
                <FormField
                  control={form.control}
                  name="service_ref"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Service</FormLabel>
                      <FormControl>
                        <ServicePicker
                          services={services}
                          loading={servicesLoading}
                          error={servicesError}
                          onRetry={onRetryServices}
                          selected={[field.value].filter(Boolean)}
                          onChange={(values) => field.onChange(values[0] ?? "")}
                        />
                      </FormControl>
                      <FormDescription className="text-[11px]">
                        Each badge shows the unit this service meters.
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="quantity"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>
                          {metric
                            ? `Free ${billingMetricLabel(metric)}`
                            : "Free units"}
                        </FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={1_000_000_000_000}
                            {...field}
                            value={
                              Number.isFinite(field.value) ? field.value : ""
                            }
                            onChange={(event) =>
                              field.onChange(event.target.valueAsNumber)
                            }
                          />
                        </FormControl>
                        <FormDescription className="text-[11px] leading-relaxed">
                          {metric
                            ? `This service is metered in ${metric}. ${
                                preview ??
                                "Enter a whole-number quantity to preview included usage"
                              }.`
                            : "Select a service to see whether included usage is measured in tokens, requests, or bytes."}
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
                          onValueChange={field.onChange}
                        >
                          <FormControl>
                            <SelectTrigger>
                              <SelectValue />
                            </SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            <SelectItem value="one_time">One time</SelectItem>
                            <SelectItem value="daily">Daily</SelectItem>
                            <SelectItem value="weekly">Weekly</SelectItem>
                            <SelectItem value="monthly">Monthly</SelectItem>
                          </SelectContent>
                        </Select>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
                <AllowanceTargetFields
                  form={form}
                  targetKind={targetKind}
                  onOwnersError={setOwnersDiscoveryFailed}
                  onOwnersLoading={setOwnersLoading}
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
                {editingAllowance ? "Save changes" : "Add included usage"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function AllowanceTargetFields({
  form,
  targetKind,
  onOwnersError,
  onOwnersLoading,
}: {
  readonly form: AllowanceFormApi;
  readonly onOwnersError: (failed: boolean) => void;
  readonly onOwnersLoading: (loading: boolean) => void;
  readonly targetKind: "all_users" | "selected_users";
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
                if (value === "all_users") form.setValue("target_user_ids", []);
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

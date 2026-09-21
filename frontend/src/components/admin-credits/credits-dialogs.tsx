import type { AllowanceBundle } from "./allowance-bundles";
import { useFieldArray, type UseFormReturn } from "react-hook-form";
import {
  billingMetricLabel,
  formatAllowancePreview,
} from "@/lib/billing-units";
import type {
  AllowanceBundleForm as AllowanceForm,
  IssueGrantForm,
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
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
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

type GrantFormApi = UseFormReturn<IssueGrantForm>;
type AllowanceFormApi = UseFormReturn<AllowanceForm>;

export function GrantDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: GrantFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly onSubmit: (value: IssueGrantForm) => Promise<void>;
}) {
  const allServices = form.watch("all_services");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>Issue credit grant</DialogTitle>
          <DialogDescription>
            Issue wallet currency to billing owners. Credits are not service
            units such as tokens, requests, or bytes.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)}>
            <DialogBody className="-mx-1 px-1">
              <div className="space-y-5 pb-1">
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="amount_credits"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Wallet credits per owner</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={1_000_000}
                            {...field}
                            onChange={(event) =>
                              field.onChange(event.target.valueAsNumber)
                            }
                          />
                        </FormControl>
                        <FormDescription className="text-[11px]">
                          A credit is wallet currency, not a metered service
                          unit.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
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
                </div>
                <RecipientTargetFields />
                <p className="text-[11px] text-muted-foreground">
                  Recipients are captured when credits are issued.
                </p>
                <FormField
                  control={form.control}
                  name="all_services"
                  render={({ field }) => (
                    <FormItem className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
                      <div>
                        <FormLabel>All services</FormLabel>
                        <p className="text-[11px] text-muted-foreground">
                          Allow this wallet balance to fund any service.
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
                          />
                        </FormControl>
                        <FormDescription className="text-[11px]">
                          Metrics identify each service&apos;s usage unit; they
                          do not change this grant&apos;s credit amount.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                ) : null}
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
            <DialogFooter className="pt-4 md:pt-4">
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" variant="primary" isLoading={pending}>
                Issue credits
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

export function AllowanceDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  editingAllowance,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: AllowanceFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly editingAllowance: AllowanceBundle | null;
  readonly onSubmit: (value: AllowanceForm) => Promise<void>;
}) {
  const serviceRef = form.watch("service_ref");
  const units = form.watch("units");
  const { fields, append, remove, replace } = useFieldArray({
    control: form.control,
    name: "units",
  });
  const selectedService = services.find(
    (service) => service.id === serviceRef || service.slug === serviceRef,
  );
  const laneMetrics =
    selectedService?.allowance_metrics ??
    (selectedService ? [selectedService.effective_platform_metric] : []);
  const available = laneMetrics.filter(
    (metric) => !units.some((unit) => unit.metric === metric),
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>
            {editingAllowance ? "Edit allowance" : "Create allowance"}
          </DialogTitle>
          <DialogDescription>
            Grant free metered service usage before wallet credits are charged.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)}>
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
                          services={
                            editingAllowance
                              ? services.filter(
                                  (s) => s.id === editingAllowance.service_id,
                                )
                              : services
                          }
                          selected={[field.value].filter(Boolean)}
                          onChange={(values) => {
                            if (editingAllowance) return;
                            const reference = values[0] ?? "";
                            field.onChange(reference);
                            const service = services.find(
                              (s) => s.id === reference || s.slug === reference,
                            );
                            replace([
                              {
                                metric:
                                  service?.allowance_metrics?.[0] ??
                                  service?.effective_platform_metric ??
                                  "tokens",
                                quantity: 1_000,
                                recurrence: "monthly",
                              },
                            ]);
                          }}
                        />
                      </FormControl>
                      <FormDescription className="text-[11px]">
                        {editingAllowance
                          ? "To change the service, create a new allowance bundle."
                          : "Choose a service, then add its free billing units."}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <div className="space-y-3">
                  {fields.map((unit, index) => (
                    <div
                      key={unit.id}
                      className="grid gap-3 rounded-lg border border-border p-3 sm:grid-cols-[1fr_1fr_1fr_auto]"
                    >
                      <FormField
                        control={form.control}
                        name={`units.${index}.metric`}
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Unit</FormLabel>
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
                                {[...new Set([field.value, ...available])].map(
                                  (metric) => (
                                    <SelectItem key={metric} value={metric}>
                                      {billingMetricLabel(metric)}
                                    </SelectItem>
                                  ),
                                )}
                              </SelectContent>
                            </Select>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <FormField
                        control={form.control}
                        name={`units.${index}.quantity`}
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Free quantity</FormLabel>
                            <FormControl>
                              <Input
                                type="number"
                                min={1}
                                max={1_000_000_000_000}
                                {...field}
                                onChange={(event) =>
                                  field.onChange(event.target.valueAsNumber)
                                }
                              />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <FormField
                        control={form.control}
                        name={`units.${index}.recurrence`}
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
                                {(
                                  [
                                    "one_time",
                                    "daily",
                                    "weekly",
                                    "monthly",
                                  ] as const
                                ).map((value) => (
                                  <SelectItem key={value} value={value}>
                                    {value === "one_time"
                                      ? "One time"
                                      : value.charAt(0).toUpperCase() +
                                        value.slice(1)}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        className="self-end"
                        disabled={fields.length === 1}
                        onClick={() => remove(index)}
                      >
                        Remove unit
                      </Button>
                    </div>
                  ))}
                  <Button
                    type="button"
                    variant="outline"
                    disabled={!available.length || fields.length >= 16}
                    onClick={() => {
                      const metric = available[0];
                      if (metric)
                        append({
                          metric,
                          quantity: 1_000,
                          recurrence: "monthly",
                        });
                    }}
                  >
                    Add unit
                  </Button>
                  <p className="text-[11px] text-muted-foreground">
                    {units
                      .map(
                        (unit) =>
                          formatAllowancePreview(
                            unit.quantity,
                            unit.metric,
                            unit.recurrence,
                          ) ?? "Enter a whole-number quantity",
                      )
                      .join(" · ")}
                  </p>
                  {editingAllowance && (
                    <p className="text-[11px] text-muted-foreground">
                      Removing a unit disables its allowance when you save.
                      Existing consumption is retained.{" "}
                      {editingAllowance.rows.some((row) => row.is_active)
                        ? "Disabled units stay disabled unless you add them again. Use Enable to restore all units."
                        : "All units are disabled, so they are loaded for editing. Saving re-enables the listed units."}
                    </p>
                  )}
                </div>
                <RecipientTargetFields />
                <p className="text-[11px] text-muted-foreground">
                  Organization and group allowances follow live membership.
                  People who leave stop receiving new free usage.
                </p>
              </div>
            </DialogBody>
            <DialogFooter className="pt-4 md:pt-4">
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={pending}
                disabled={
                  (!!editingAllowance && !form.formState.isDirty) || !serviceRef
                }
              >
                {editingAllowance ? "Save changes" : "Create allowance"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

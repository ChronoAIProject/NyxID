import type { UseFormReturn } from "react-hook-form";
import {
  billingMetricLabel,
  formatAllowancePreview,
  resolveServiceBillingMetric,
} from "@/lib/billing-units";
import type {
  AllowanceForm,
  IssueGrantForm,
  UsageAllowance,
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
import { ServicePicker, UserPicker } from "./credit-pickers";

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
  const targetKind = form.watch("target_kind");
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
                <GrantTargetFields form={form} targetKind={targetKind} />
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
  readonly editingAllowance: UsageAllowance | null;
  readonly onSubmit: (value: AllowanceForm) => Promise<void>;
}) {
  const targetKind = form.watch("target_kind");
  const serviceRef = form.watch("service_ref");
  const quantity = form.watch("quantity");
  const recurrence = form.watch("recurrence");
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
                          services={services}
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
                            onChange={(event) =>
                              field.onChange(event.target.valueAsNumber)
                            }
                          />
                        </FormControl>
                        <FormDescription className="text-[11px] leading-relaxed">
                          {metric
                            ? `This service is metered in ${metric}. ${
                                preview ??
                                "Enter a whole-number quantity to preview the allowance"
                              }.`
                            : "Select a service to see whether its allowance is measured in tokens, requests, or bytes."}
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
                <AllowanceTargetFields form={form} targetKind={targetKind} />
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
                {editingAllowance ? "Save changes" : "Create allowance"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function GrantTargetFields({
  form,
  targetKind,
}: {
  readonly form: GrantFormApi;
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
                <UserPicker selected={field.value} onChange={field.onChange} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}

function AllowanceTargetFields({
  form,
  targetKind,
}: {
  readonly form: AllowanceFormApi;
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
                <UserPicker selected={field.value} onChange={field.onChange} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}

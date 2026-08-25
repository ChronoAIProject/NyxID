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

type ScheduleFormApi = UseFormReturn<ScheduleForm>;

export function ScheduleDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  editingSchedule,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: ScheduleFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly editingSchedule: CreditSchedule | null;
  readonly onSubmit: (value: ScheduleForm) => Promise<void>;
}) {
  const expiry = form.watch("expiry");
  const targetKind = form.watch("target_kind");
  const allServices = form.watch("all_services");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent scrollMode="body" className="md:max-w-2xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>
            {editingSchedule
              ? "Edit credit schedule"
              : "Create credit schedule"}
          </DialogTitle>
          <DialogDescription>
            Disburse wallet credits on a UTC cycle. Credits are wallet currency,
            not service units such as tokens, requests, or bytes.
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
                          Minted as an ordinary promotional credit grant each
                          period.
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
                        {editingSchedule ? (
                          <FormDescription className="text-[11px]">
                            Pause this schedule and create another to change its
                            recurrence.
                          </FormDescription>
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
                            description="From disbursement"
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
                            value={Number.isNaN(field.value) ? "" : field.value}
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

                <TargetFields form={form} targetKind={targetKind} />

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
            <DialogFooter className="pt-4 md:pt-4">
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" variant="primary" isLoading={pending}>
                {editingSchedule ? "Save changes" : "Create schedule"}
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
}: {
  readonly form: ScheduleFormApi;
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

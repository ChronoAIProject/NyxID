import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useFieldArray, useFormContext } from "react-hook-form";
import type { AllocationForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { Button } from "@/components/ui/button";
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { ServicePicker } from "./credit-pickers";

export function ServiceLimitsEditor({
  services,
  disabled = false,
}: {
  readonly services: readonly DownstreamService[];
  readonly disabled?: boolean;
}) {
  useUiTranslation();
  const form = useFormContext<AllocationForm>();
  const rules = useFieldArray({
    control: form.control,
    name: "service_limits",
  });
  return (
    <details className="rounded-lg border border-border p-3" open>
      <summary className="cursor-pointer type-card-title">{translateUi("Service limits (optional)")}</summary>
      <p className="my-3 text-14 text-muted-foreground">{translateUi("Limits share the allocation total. A group of services shares one ceiling. They do not reserve separate funds. Other eligible services remain unrestricted, and other allocations or purchased credits may pay any excess.")}</p>
      <fieldset disabled={disabled} className="space-y-4">
        {rules.fields.map((rule, index) => (
          <div
            key={rule.id}
            className="space-y-3 rounded-md border border-border p-3"
          >
            <FormField
              control={form.control}
              name={`service_limits.${index}.label`}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Group label ")}{index + 1}{translateUi(" (optional)")}</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name={`service_limits.${index}.service_refs`}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Services sharing limit ")}{index + 1}</FormLabel>
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
            <FormField
              control={form.control}
              name={`service_limits.${index}.amount`}
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Credit ceiling ")}{index + 1}</FormLabel>
                  <FormControl>
                    <Input inputMode="decimal" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => rules.remove(index)}
            >{translateUi("Remove limit ")}{index + 1}
            </Button>
          </div>
        ))}
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={disabled || rules.fields.length >= 20}
          onClick={() =>
            rules.append({ label: "", service_refs: [], amount: "" })
          }
        >{translateUi("Add service limit")}</Button>
      </fieldset>
    </details>
  );
}

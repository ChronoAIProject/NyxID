import type { ComponentProps } from "react";
import { FormItem, FormLabel, FormDescription, FormMessage } from "./form";
import { GroupedFooter, GroupedRow } from "./grouped-list";
import { cn } from "@/lib/utils";

/** A form item retains its field IDs while using the shared row and footer slots. */
export function GroupedField({
  inline = false,
  className,
  ...props
}: ComponentProps<typeof FormItem> & { readonly inline?: boolean }) {
  return (
    <GroupedRow asChild>
      <FormItem
        className={cn(
          inline
            ? "space-y-0 [&>label]:flex-1 [&>label]:text-14 [&>label]:font-medium [&>label]:text-foreground"
            : "block space-y-0.5",
          className,
        )}
        {...props}
      />
    </GroupedRow>
  );
}

export function GroupedLabel({
  className,
  ...props
}: ComponentProps<typeof FormLabel>) {
  return (
    <FormLabel
      className={cn("text-13 font-normal text-text-tertiary", className)}
      {...props}
    />
  );
}

export function GroupedDescription(
  props: ComponentProps<typeof FormDescription>,
) {
  return (
      <FormDescription
        {...props}
        className={cn("basis-full text-13 font-normal text-text-tertiary", props.className)}
      />
  );
}

export function GroupedMessage(props: ComponentProps<typeof FormMessage>) {
  return (
    <GroupedFooter>
      <FormMessage {...props} className={cn("text-13", props.className)} />
    </GroupedFooter>
  );
}

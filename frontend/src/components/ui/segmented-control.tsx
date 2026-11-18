import type { ComponentProps } from "react";
import * as Tabs from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";

export function SegmentedControl(props: ComponentProps<typeof Tabs.Root>) {
  return <Tabs.Root {...props} />;
}

export function SegmentedControlList({
  className,
  ...props
}: ComponentProps<typeof Tabs.List>) {
  return (
    <Tabs.List
      className={cn(
        "flex w-full flex-wrap gap-1 rounded-lg bg-muted p-1 text-14",
        className,
      )}
      {...props}
    />
  );
}

export function SegmentedControlTrigger({
  className,
  ...props
}: ComponentProps<typeof Tabs.Trigger>) {
  return (
    <Tabs.Trigger
      className={cn(
        "min-h-8 flex-auto shrink-0 rounded-md px-1.5 py-1.5 type-row-title text-text-tertiary transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm",
        className,
      )}
      {...props}
    />
  );
}

export function SegmentedControlContent({
  className,
  ...props
}: ComponentProps<typeof Tabs.Content>) {
  return (
    <Tabs.Content
      className={cn("space-y-6 outline-none", className)}
      {...props}
    />
  );
}

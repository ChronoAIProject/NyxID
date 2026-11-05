import * as React from "react";
import { TEXT_STYLES } from "@/lib/typography";
import { cn } from "@/lib/utils";
import { ChevronDown } from "lucide-react";

/** Native options for settings that benefit from the platform picker. */
const NativeSelect = React.forwardRef<
  HTMLSelectElement,
  React.SelectHTMLAttributes<HTMLSelectElement>
>(({ className, ...props }, ref) => (
  <span className="relative block min-w-0 max-w-full">
    <select
      ref={ref}
      data-slot="native-select"
      className={cn(
        `min-h-input block w-full min-w-0 max-w-full appearance-none rounded-lg border border-input bg-background px-3 py-1.5 pr-[calc(1.125em+var(--spacing)*5)] ${TEXT_STYLES.input} text-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50`,
        className,
      )}
      {...props}
    />
    <ChevronDown aria-hidden="true" data-ui-icon="" className="pointer-events-none absolute right-3 top-1/2 size-[1.125em] -translate-y-1/2 text-text-tertiary" />
  </span>
));
NativeSelect.displayName = "NativeSelect";

export { NativeSelect };

import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

/* ── VoidPortal Badge Variants ── */
const badgeVariants = cva(
  "inline-flex items-center rounded-[10px] border px-2.5 py-1 text-[10px] font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      variant: {
        default:
          "border-primary/30 bg-primary/15 text-void-300",
        secondary:
          "border-transparent bg-muted text-muted-foreground",
        destructive:
          "border-destructive/30 bg-destructive/15 text-destructive",
        outline:
          "border-border text-foreground",
        success:
          "border-success/40 bg-transparent text-success",
        warning:
          "border-warning/40 bg-transparent text-warning",
        info:
          "border-info/40 bg-transparent text-info",
        accent:
          "border-primary/40 bg-transparent text-void-400",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends
    React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <div className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };

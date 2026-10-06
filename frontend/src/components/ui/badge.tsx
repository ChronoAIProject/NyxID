import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

/* ── NyxID Badge Variants ── */
const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2 py-0.5 text-10 font-medium transition-colors duration-200 focus:outline-none",
  {
    variants: {
      // Dark (base, unprefixed) is the primary surface and stays as the tuned tint.
      // Light mode (`light:` variant) keeps the soft tint but swaps the pale text
      // for the deep 700 ramp, which is what carries contrast on a light canvas.
      variant: {
        default:
          "border-nyx-500/30 bg-nyx-500/15 text-nyx-200 light:border-nyx-500/20 light:bg-nyx-500/10 light:text-nyx-700",
        secondary:
          "border-transparent bg-muted text-muted-foreground light:border-hairline light:bg-muted",
        destructive:
          "border-destructive/30 bg-destructive/15 text-destructive light:border-destructive/20 light:bg-destructive/10 light:text-red-700",
        success:
          "border-success/30 bg-success/10 text-success light:border-success/20 light:bg-success/10 light:text-emerald-700",
        warning:
          "border-warning/30 bg-warning/10 text-warning light:border-warning/25 light:bg-warning/10 light:text-amber-700",
        info: "border-info/30 bg-info/10 text-info light:border-info/20 light:bg-info/10 light:text-blue-700",
        accent:
          "border-nyx-500/30 bg-nyx-500/10 text-nyx-secondary-400 light:border-nyx-secondary-500/20 light:bg-nyx-secondary-500/10 light:text-nyx-secondary-700",
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

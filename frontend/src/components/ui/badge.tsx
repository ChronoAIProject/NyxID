import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

/* ── NyxID Badge Variants ── */
const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-medium transition-colors duration-200 focus:outline-none",
  {
    variants: {
      // Dark (base, unprefixed) is the primary surface and stays as the tuned tint.
      // Light mode deviates to a solid fill (`light:` variant) — the tint loses
      // contrast on a light canvas, so color-carrying badges become solid + white
      // text there. `secondary` is a neutral chip and reads well in both, so it is
      // left untouched.
      variant: {
        default:
          "border-nyx-500/30 bg-nyx-500/15 text-nyx-200 light:border-transparent light:bg-nyx-500 light:text-white",
        secondary:
          "border-transparent bg-muted text-muted-foreground light:border-muted-foreground light:bg-transparent",
        destructive:
          "border-destructive/30 bg-destructive/15 text-destructive light:border-transparent light:bg-destructive light:text-white",
        success:
          "border-success/30 bg-success/10 text-success light:border-transparent light:bg-success light:text-white",
        warning:
          "border-warning/30 bg-warning/10 text-warning light:border-transparent light:bg-warning light:text-white",
        info: "border-info/30 bg-info/10 text-info light:border-transparent light:bg-info light:text-white",
        accent:
          "border-nyx-500/30 bg-nyx-500/10 text-nyx-secondary-400 light:border-transparent light:bg-nyx-secondary-500 light:text-white",
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

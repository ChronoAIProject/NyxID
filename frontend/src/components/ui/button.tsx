import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";
import { Loader2 } from "lucide-react";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-lg text-[12px] font-medium transition-all duration-200 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-3 [&_svg]:shrink-0 cursor-pointer",
  {
    variants: {
      variant: {
        default:
          "border border-white/[0.08] bg-white/[0.04] text-foreground hover:border-white/[0.15] hover:bg-white/[0.06]",
        destructive:
          "border border-destructive/30 bg-destructive/10 text-destructive hover:border-destructive/50 hover:bg-destructive/15",
        outline:
          "border border-white/[0.08] bg-transparent text-muted-foreground hover:border-white/[0.15] hover:text-foreground",
        secondary:
          "border border-white/[0.08] bg-white/[0.04] text-muted-foreground hover:border-white/[0.15] hover:text-foreground",
        ghost:
          "text-muted-foreground hover:bg-white/[0.04] hover:text-foreground",
        link: "text-nyx-secondary-400 underline-offset-4 hover:underline",
        primary:
          "nyx-gradient-vivid text-white shadow-[0_0_12px_rgba(90,42,241,0.25)] hover:shadow-[0_0_18px_rgba(90,42,241,0.35)] hover:brightness-110",
      },
      size: {
        default: "h-8 px-3",
        sm: "h-7 px-2.5",
        lg: "h-9 px-4",
        icon: "h-8 w-8",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export function ButtonIcon({ children, className, variant }: { readonly children: React.ReactNode; readonly className?: string; readonly variant?: "default" | "destructive" | "primary" }) {
  return (
    <span className={cn(
      "flex h-[18px] w-[18px] items-center justify-center rounded-[4px]",
      variant === "destructive"
        ? "border border-destructive/20 bg-destructive/10"
        : variant === "primary"
          ? "border border-white/20 bg-white/10"
          : "border border-white/[0.08] bg-white/[0.04]",
      className,
    )}>
      {children}
    </span>
  );
}

export interface ButtonProps
  extends
    React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  readonly asChild?: boolean;
  readonly isLoading?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      className,
      variant,
      size,
      asChild = false,
      isLoading = false,
      children,
      disabled,
      ...props
    },
    ref,
  ) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        disabled={disabled ?? isLoading}
        {...props}
      >
        {isLoading ? (
          <>
            <Loader2 className="animate-spin" />
            {children}
          </>
        ) : (
          children
        )}
      </Comp>
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };

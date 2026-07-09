import * as React from "react";
import { cn } from "@/lib/utils";

/* ── NyxID Input ── */
const Input = React.forwardRef<
  HTMLInputElement,
  React.InputHTMLAttributes<HTMLInputElement>
>(({ className, type, ...props }, ref) => {
  return (
    <input
      type={type}
      className={cn(
        // aria-invalid keeps the error border while errored — even when
        // focused — for every consumer that sets it (FormControl does so
        // automatically for FormField-driven inputs).
        "flex h-8 w-full rounded-lg border border-input bg-transparent px-3 py-1.5 text-[12px] text-foreground transition-colors duration-200 file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-text-tertiary focus-visible:outline-none focus-visible:border-white/[0.15] aria-invalid:border-destructive aria-invalid:focus-visible:border-destructive disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      ref={ref}
      {...props}
    />
  );
});
Input.displayName = "Input";

export { Input };

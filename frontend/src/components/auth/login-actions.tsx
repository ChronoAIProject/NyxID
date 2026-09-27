import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";

export function LoginActions({
  children,
  onDeny,
  disabled,
  paired = false,
}: {
  children: ReactNode;
  onDeny: () => void;
  disabled: boolean;
  paired?: boolean;
}) {
  return (
    <div
      className={
        paired
          ? "grid grid-cols-2 items-stretch gap-3 [&>button]:h-auto [&>button]:min-h-9 [&>button]:min-w-0 [&>button]:whitespace-normal [&>button]:rounded-md [&>button]:px-3 [&>button]:py-2"
          : "flex flex-col items-start gap-1 [&>button:first-child]:h-auto [&>button:first-child]:min-h-9 [&>button:first-child]:w-full [&>button:first-child]:whitespace-normal [&>button:first-child]:rounded-md [&>button:first-child]:px-3 [&>button:first-child]:py-2"
      }
    >
      {children}
      {paired ? (
        <Button
          type="button"
          variant="outline"
          disabled={disabled}
          onClick={onDeny}
        >
          Deny request
        </Button>
      ) : (
        <button
          type="button"
          disabled={disabled}
          onClick={onDeny}
          className="min-h-8 rounded-sm text-left text-[11px] text-muted-foreground hover:text-foreground hover:underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none disabled:opacity-50"
        >
          Cancel request
        </button>
      )}
    </div>
  );
}

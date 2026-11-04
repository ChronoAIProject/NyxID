import type { ReactNode } from "react";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

/**
 * Labelled two-up radio group of choice cards, the shared shape for "which
 * key / which OAuth app" decisions on the dashboard, connector page and CLI
 * wizard.
 */
export function ChoiceCardGroup({
  label,
  ariaLabel,
  children,
  footer,
}: {
  readonly label: string;
  readonly ariaLabel: string;
  readonly children: ReactNode;
  readonly footer?: ReactNode;
}) {
  return (
    <div className="space-y-3">
      <Label>{label}</Label>
      <div
        role="radiogroup"
        aria-label={ariaLabel}
        className="grid grid-cols-2 gap-3"
      >
        {children}
      </div>
      {footer}
    </div>
  );
}

/** One card in a ChoiceCardGroup. Forwards a tooltip trigger's ref and handlers. */
export function ChoiceCard({
  checked,
  disabled = false,
  onSelect,
  icon,
  title,
  description,
  onClick,
  ...triggerProps
}: {
  readonly checked: boolean;
  readonly disabled?: boolean;
  readonly onSelect: () => void;
  readonly icon: ReactNode;
  readonly title: string;
  readonly description: ReactNode;
} & Omit<React.ComponentPropsWithRef<"button">, "title">) {
  return (
    <button
      {...triggerProps}
      type="button"
      role="radio"
      aria-checked={checked}
      disabled={disabled}
      onClick={(event) => {
        onClick?.(event);
        onSelect();
      }}
      className={cn(
        "flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors duration-300 disabled:cursor-not-allowed disabled:opacity-60",
        checked
          ? "border-primary bg-primary/5"
          : "border-border hover:border-hairline-strong",
      )}
    >
      {icon}
      <span className="text-13 font-medium">{title}</span>
      <span className="text-12 text-muted-foreground">{description}</span>
    </button>
  );
}

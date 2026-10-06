import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";

export const LOGIN_PROVIDER_ROW_CLASS =
  "flex min-h-[46px] w-full cursor-pointer items-center gap-3 rounded-lg border border-border bg-background px-4 py-2 text-left text-[0.84375rem] font-medium text-foreground transition-colors hover:border-border/80 hover:bg-overlay disabled:cursor-not-allowed disabled:opacity-60";

export function LoginProviderRow({
  icon,
  label,
  disabled,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={LOGIN_PROVIDER_ROW_CLASS}
    >
      <span
        className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-overlay-strong"
        aria-hidden="true"
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1">{label}</span>
      <ChevronRight
        className="size-4 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />
    </button>
  );
}

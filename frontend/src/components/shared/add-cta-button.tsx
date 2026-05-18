import { Plus } from "lucide-react";

interface AddCtaButtonProps {
  readonly label: string;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly icon?: React.ComponentType<{ className?: string }>;
}

export function AddCtaButton({ label, onClick, disabled = false, icon: Icon = Plus }: AddCtaButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex h-8 items-center gap-2 rounded-lg border border-white/[0.08] px-3 text-[12px] text-text-tertiary transition-all duration-300 hover:border-white/[0.15] hover:text-muted-foreground disabled:pointer-events-none disabled:opacity-40"
    >
      <span className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] border border-white/[0.08] bg-white/[0.04]">
        <Icon className="h-3 w-3" />
      </span>
      {label}
    </button>
  );
}

import { Plus } from "lucide-react";
import { Button, ButtonIcon } from "@/components/ui/button";

interface AddCtaButtonProps {
  readonly label: string;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly icon?: React.ComponentType<{ className?: string }>;
  /**
   * "primary" (default) → the goal-completing CTA on this page. Renders
   * as a full primary button using the shared `<Button variant="primary">`
   * — same component AiSetupCard, ApprovalsCard, etc. use, so visual
   * hierarchy stays consistent across the app.
   *
   * "subtle" → secondary additions that explicitly should NOT compete
   * with another CTA on the same surface (e.g., a "+" affordance below
   * a list when the page's main CTA lives elsewhere). Renders as the
   * original ghost-styled chip.
   */
  readonly variant?: "primary" | "subtle";
}

export function AddCtaButton({
  label,
  onClick,
  disabled = false,
  icon: Icon = Plus,
  variant = "primary",
}: AddCtaButtonProps) {
  if (variant === "primary") {
    return (
      <Button
        variant="primary"
        size="lg"
        onClick={onClick}
        disabled={disabled}
      >
        <ButtonIcon variant="primary">
          <Icon className="h-3.5 w-3.5" />
        </ButtonIcon>
        {label}
      </Button>
    );
  }

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

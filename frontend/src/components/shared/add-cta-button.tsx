import { Plus } from "lucide-react";
import { Button, ButtonIcon } from "@/components/ui/button";

interface AddCtaButtonProps {
  readonly label: string;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly icon?: React.ComponentType<{ className?: string }>;
  /**
   * "primary" → the goal-completing CTA on this page (e.g., "Add Service"
   * on /keys, "Create Agent Key" on the agent-keys tab). Renders as a
   * full primary button using the shared `<Button variant="primary">`.
   *
   * "subtle" (default) → secondary additions on pages that aren't the
   * user's main goal (e.g., "Add Route" on a channel-bot detail page).
   * Renders as the original ghost-styled chip so it doesn't compete with
   * the page's primary affordance.
   */
  readonly variant?: "primary" | "subtle";
}

export function AddCtaButton({
  label,
  onClick,
  disabled = false,
  icon: Icon = Plus,
  variant = "subtle",
}: AddCtaButtonProps) {
  if (variant === "primary") {
    return (
      <Button
        variant="primary"
        onClick={onClick}
        disabled={disabled}
      >
        <ButtonIcon variant="primary">
          <Icon className="h-3 w-3" />
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

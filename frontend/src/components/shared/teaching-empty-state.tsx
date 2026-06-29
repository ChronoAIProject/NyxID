import type { ComponentType, ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface CtaActionable {
  readonly label: string;
  readonly onClick: () => void;
}
interface CtaLink {
  readonly label: string;
  readonly href: string;
}
type Cta = CtaActionable | CtaLink;

interface JumpStart {
  readonly label: string;
  readonly onClick: () => void;
  readonly icon?: ReactNode;
}

interface TeachingEmptyStateProps {
  readonly icon: ComponentType<{ readonly className?: string }>;
  /** Bold one-liner. Foreground contrast — what the section IS / why it's empty. */
  readonly title: string;
  /** One-sentence why-care. Muted contrast — the value the user gets by filling it. */
  readonly description: string;
  readonly primaryCta: Cta;
  /**
   * Optional "Or start with X" chips for catalog-backed surfaces (the keys
   * page, etc.). Capped at 5 to keep the visual budget bounded. AGY's
   * Wave B addition — for empty surfaces where the user genuinely doesn't
   * know what their first move should be, "Start with OpenAI" is more
   * useful than a bare "Create" button.
   */
  readonly catalogJumpStarts?: ReadonlyArray<JumpStart>;
  readonly className?: string;
}

/**
 * Canonical empty-state structure for list pages. Wave A patched
 * contrast (`text-muted-foreground/30` → default muted) across ~26
 * pages; this primitive locks the *shape* (icon → title → description
 * → primary CTA → optional jump-starts) so future empty states don't
 * drift back into the bespoke shapes Wave A patched. Wave B item B.5.
 *
 * Title uses `text-foreground` and description uses `text-muted-foreground`
 * — both readable contrasts, not the `/30` ghost-text that originally
 * made these surfaces look like decorative filler.
 */
export function TeachingEmptyState({
  icon: Icon,
  title,
  description,
  primaryCta,
  catalogJumpStarts,
  className,
}: TeachingEmptyStateProps) {
  const ctaButton =
    "onClick" in primaryCta ? (
      <Button variant="primary" size="lg" onClick={primaryCta.onClick}>
        {primaryCta.label}
      </Button>
    ) : (
      <Button variant="primary" size="lg" asChild>
        <a href={primaryCta.href}>{primaryCta.label}</a>
      </Button>
    );

  const jumpStarts = catalogJumpStarts?.slice(0, 5) ?? [];

  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 py-12 text-center",
        className,
      )}
    >
      <Icon className="h-12 w-12 text-muted-foreground" />
      <h3 className="text-[14px] font-semibold text-foreground">{title}</h3>
      <p className="max-w-md text-[12px] text-muted-foreground">{description}</p>
      <div className="mt-2">{ctaButton}</div>
      {jumpStarts.length > 0 ? (
        <div className="mt-3 flex flex-wrap items-center justify-center gap-2">
          <span className="text-[11px] text-muted-foreground">Or start with:</span>
          {jumpStarts.map((j) => (
            <Button
              key={j.label}
              variant="outline"
              size="sm"
              onClick={j.onClick}
              className="gap-1.5"
            >
              {j.icon}
              {j.label}
            </Button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

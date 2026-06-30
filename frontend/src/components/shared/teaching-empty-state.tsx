import type { ComponentType, ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { cn } from "@/lib/utils";

interface CtaActionable {
  readonly label: string;
  readonly onClick: () => void;
  /**
   * Lucide icon to prefix on the CTA button — defaults to `Plus` via
   * `AddCtaButton`. Pass a different icon when the action isn't "create
   * a new thing" (e.g. `Plug` for connect, `Download` for import).
   */
  readonly icon?: ComponentType<{ className?: string }>;
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
  /**
   * Override the default illustration sizing/contrast. Default matches the
   * pre-primitive bespoke shapes Wave A patched (`h-48 w-48` at `/40`
   * opacity) so converting a surface to the primitive doesn't shrink its
   * hero illustration. Pass a smaller class set when the empty state lives
   * inside a sub-card and the full-page illustration would crowd it out.
   */
  readonly iconClassName?: string;
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

const DEFAULT_ICON_CLASS = "h-48 w-48 text-muted-foreground/40";

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
 *
 * Visual defaults intentionally match the pre-primitive bespoke shapes
 * (`h-48 w-48` illustration at `/40`, `text-[15px]` title, `AddCtaButton`
 * for actionable CTAs) so converting a surface to the primitive preserves
 * its existing hierarchy. Pass `iconClassName` to opt into a smaller icon
 * when the empty state lives inside a constrained sub-card.
 */
export function TeachingEmptyState({
  icon: Icon,
  iconClassName,
  title,
  description,
  primaryCta,
  catalogJumpStarts,
  className,
}: TeachingEmptyStateProps) {
  const ctaButton =
    "onClick" in primaryCta ? (
      // AddCtaButton renders <Button variant="primary" size="lg"> with a
      // Plus-prefixed ButtonIcon — same brand treatment as the toolbar
      // "Add"/"Connect" buttons across the app, so empty-state CTAs and
      // populated-state CTAs look like siblings instead of strangers.
      <AddCtaButton
        label={primaryCta.label}
        onClick={primaryCta.onClick}
        icon={primaryCta.icon}
      />
    ) : (
      <Button variant="primary" size="lg" asChild>
        <a href={primaryCta.href}>{primaryCta.label}</a>
      </Button>
    );

  const jumpStarts = catalogJumpStarts?.slice(0, 5) ?? [];

  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-4 py-12 text-center",
        className,
      )}
    >
      <Icon className={cn(iconClassName ?? DEFAULT_ICON_CLASS)} />
      <div className="space-y-1.5 max-w-md">
        <p className="text-[15px] font-semibold text-foreground">{title}</p>
        <p className="text-[12px] text-muted-foreground leading-relaxed">
          {description}
        </p>
      </div>
      {ctaButton}
      {jumpStarts.length > 0 ? (
        <div className="mt-1 flex flex-wrap items-center justify-center gap-2">
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

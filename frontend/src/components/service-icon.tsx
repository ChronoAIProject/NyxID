import { SERVICE_ICONS, FallbackIcon } from "@/components/service-icons";
import { cn } from "@/lib/utils";

/**
 * Fixed icon-size scale for service brand glyphs, anchored to the app's
 * icon conventions (DESIGN.md): nav icons 16px, logo 20px, PageHeader
 * leading slot 32px. Surfaces pick the token that fits; sizing stays
 * consistent and only changes by editing this scale. Never hand-roll
 * `h-[n]` on a service icon — add a token here if a new size is genuinely
 * needed.
 */
export type ServiceIconSize = "xs" | "sm" | "md" | "lg";

// Sizes are `!important` on purpose: some shadcn containers (e.g.
// `DropdownMenuItem`, `Button`) ship a blanket `[&_svg]:h-3.5 w-3.5` that
// would otherwise override a service icon dropped inside them. As the
// design-system primitive, ServiceIcon must own its own box everywhere.
const SIZE_CLASS: Readonly<Record<ServiceIconSize, string>> = {
  xs: "!h-4 !w-4", // 16px — dense rows: selects, dropdowns, checkbox lists
  sm: "!h-5 !w-5", // 20px — list/table rows, compact cards
  md: "!h-6 !w-6", // 24px — card headers, detail rows
  lg: "!h-8 !w-8", // 32px — page-header leading / detail hero
};

/**
 * The app-facing brand icon for an AI service / proxy target. Config-driven
 * over the design-owned glyph registry (`@/components/service-icons`):
 * give it the catalog slug and a size token, e.g.
 * `<ServiceIcon slug="api-reddit" size="sm" />`.
 *
 * - Known slug (`api-reddit`, `llm-openai`, …) → its brand glyph, sized to
 *   the token (badged composites included).
 * - Unknown slug → the generic globe fallback.
 * - Null / absent slug (custom service with no catalog backing) → nothing,
 *   so rows stay clean and don't reserve space.
 */
export function ServiceIcon({
  slug,
  size = "sm",
  className,
}: {
  readonly slug?: string | null;
  readonly size?: ServiceIconSize;
  readonly className?: string;
}) {
  if (!slug) return null;
  const Glyph = SERVICE_ICONS[slug] ?? FallbackIcon;
  return (
    <Glyph
      className={cn(
        SIZE_CLASS[size],
        "shrink-0 text-muted-foreground",
        className,
      )}
    />
  );
}

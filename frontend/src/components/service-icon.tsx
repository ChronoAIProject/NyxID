import { SERVICE_ICONS, FallbackIcon } from "@/components/service-icons";
import { cn } from "@/lib/utils";
import { useState } from "react";

/**
 * Fixed icon-size scale for service brand glyphs, anchored to the app's
 * icon conventions (DESIGN.md): nav icons 16px, logo 20px, PageHeader
 * leading slot 32px. Surfaces pick the token that fits; sizing stays
 * consistent and only changes by editing this scale. Never hand-roll
 * `h-[n]` on a service icon — add a token here if a new size is genuinely
 * needed.
 */
export type ServiceIconSize = "2xs" | "xs" | "sm" | "md" | "lg" | "xl";

// Sizes are `!important` on purpose: some shadcn containers (e.g.
// `DropdownMenuItem`, `Button`) ship a blanket `[&_svg]:h-3.5 w-3.5` that
// would otherwise override a service icon dropped inside them. As the
// design-system primitive, ServiceIcon must own its own box everywhere.
const SIZE_CLASS: Readonly<Record<ServiceIconSize, string>> = {
  "2xs": "!h-3.5 !w-3.5", // 14px — inline checkbox rows + dropdown/select items
  xs: "!h-4 !w-4", // 16px — dense rows
  sm: "!h-5 !w-5", // 20px — list/table rows, compact cards
  md: "!h-6 !w-6", // 24px — card headers, detail rows
  lg: "!h-8 !w-8", // 32px — page-header leading / detail hero
  xl: "!h-9 !w-9", // 36px — connection identity circles
};

function safeIconUrl(value: string): string | null {
  if (value.length > 2048 || value.includes("#")) return null;
  try {
    const url = new URL(value);
    if (
      (url.protocol !== "http:" && url.protocol !== "https:") ||
      !url.hostname ||
      url.username ||
      url.password
    ) {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}

/**
 * The app-facing brand icon for an AI service / proxy target. Config-driven
 * over the design-owned glyph registry (`@/components/service-icons`):
 * give it the catalog slug and a size token, e.g.
 * `<ServiceIcon slug="api-reddit" size="sm" />`.
 *
 * - `iconUrl` → a user-selected image, with the slug glyph as fallback.
 * - Known slug (`api-reddit`, `llm-openai`, …) → its brand glyph, sized to
 *   the token (badged composites included).
 * - Unknown slug → the generic globe fallback.
 * - Null / absent slug and icon URL → nothing.
 */
export function ServiceIcon({
  slug,
  iconUrl,
  size = "sm",
  className,
}: {
  readonly slug?: string | null;
  readonly iconUrl?: string | null;
  readonly size?: ServiceIconSize;
  readonly className?: string;
}) {
  const imageUrl = iconUrl ? safeIconUrl(iconUrl) : null;
  if (imageUrl) {
    return (
      <ServiceIconImage
        key={imageUrl}
        slug={slug}
        iconUrl={imageUrl}
        size={size}
        className={className}
      />
    );
  }
  if (!slug && !iconUrl) return null;
  const Glyph = SERVICE_ICONS[slug ?? "custom"] ?? FallbackIcon;
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

function ServiceIconImage({
  slug,
  iconUrl,
  size,
  className,
}: {
  readonly slug?: string | null;
  readonly iconUrl: string;
  readonly size: ServiceIconSize;
  readonly className?: string;
}) {
  const [failed, setFailed] = useState(false);
  if (failed) {
    return <ServiceIcon slug={slug ?? "custom"} size={size} className={className} />;
  }
  return (
    <img
      src={iconUrl}
      alt=""
      aria-hidden="true"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
      className={cn(SIZE_CLASS[size], "shrink-0 object-contain", className)}
    />
  );
}

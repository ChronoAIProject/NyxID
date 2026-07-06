import type { SVGProps } from "react";
import { Bot } from "lucide-react";
import { ServiceIcon, type ServiceIconSize } from "@/components/service-icon";
import { cn } from "@/lib/utils";

/**
 * Agent platforms (`claude-code`, `cursor`, `codex`, `openclaw`,
 * `generic`) that map onto a brand glyph in the service-icon registry.
 * The rest render a dedicated glyph (`cursor`) or a Lucide fallback.
 */
const PLATFORM_SLUG: Readonly<Record<string, string>> = {
  "claude-code": "llm-anthropic",
  codex: "llm-openai-codex",
  openclaw: "llm-openclaw",
};

// Mirrors `service-icon.tsx`'s scale exactly, `!important` and all, so a
// platform icon lines up with a service icon on the same surface and stays
// robust inside containers that ship a blanket `[&_svg]` size (e.g.
// `DropdownMenuItem`).
const SIZE_CLASS: Readonly<Record<ServiceIconSize, string>> = {
  "2xs": "!h-3.5 !w-3.5",
  xs: "!h-4 !w-4",
  sm: "!h-5 !w-5",
  md: "!h-6 !w-6",
  lg: "!h-8 !w-8",
};

/** Cursor editor mark (brand glyph, `currentColor`). */
function CursorGlyph(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      fillRule="evenodd"
      aria-hidden="true"
      {...props}
    >
      <path d="M22.106 5.68L12.5.135a.998.998 0 00-.998 0L1.893 5.68a.84.84 0 00-.419.726v11.186c0 .3.16.577.42.727l9.607 5.547a.999.999 0 00.998 0l9.608-5.547a.84.84 0 00.42-.727V6.407a.84.84 0 00-.42-.726zm-.603 1.176L12.228 22.92c-.063.108-.228.064-.228-.061V12.34a.59.59 0 00-.295-.51l-9.11-5.26c-.107-.062-.063-.228.062-.228h18.55c.264 0 .428.286.296.514z" />
    </svg>
  );
}

/**
 * Icon for an agent platform, sized off the same `ServiceIconSize` scale
 * as `<ServiceIcon>`. Brand-mapped platforms render their registry glyph,
 * `cursor` renders its own brand mark, and anything else (`generic` or an
 * unknown platform) gets a Lucide fallback. A null / absent / "none"
 * platform renders nothing.
 */
export function PlatformIcon({
  platform,
  size = "xs",
  className,
}: {
  readonly platform?: string | null;
  readonly size?: ServiceIconSize;
  readonly className?: string;
}) {
  if (!platform || platform === "__none__") return null;
  const slug = PLATFORM_SLUG[platform];
  if (slug) {
    return <ServiceIcon slug={slug} size={size} className={className} />;
  }
  const glyphClass = cn(
    SIZE_CLASS[size],
    "shrink-0 text-muted-foreground",
    className,
  );
  if (platform === "cursor") {
    return <CursorGlyph className={glyphClass} />;
  }
  return <Bot className={glyphClass} aria-hidden="true" />;
}

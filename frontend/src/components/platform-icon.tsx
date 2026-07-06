import { Bot, MousePointer2 } from "lucide-react";
import { ServiceIcon, type ServiceIconSize } from "@/components/service-icon";
import { cn } from "@/lib/utils";

/**
 * Agent platforms (`claude-code`, `cursor`, `codex`, `openclaw`,
 * `generic`) that map onto a brand glyph in the service-icon registry.
 * The rest fall back to a recognizable Lucide mark.
 */
const PLATFORM_SLUG: Readonly<Record<string, string>> = {
  "claude-code": "llm-anthropic",
  codex: "llm-openai-codex",
  openclaw: "llm-openclaw",
};

// Sizes match the ServiceIcon token scale so a platform icon lines up with
// a service icon on the same surface.
const LUCIDE_SIZE: Readonly<Record<ServiceIconSize, string>> = {
  xs: "h-4 w-4",
  sm: "h-5 w-5",
  md: "h-6 w-6",
  lg: "h-8 w-8",
};

/**
 * Icon for an agent platform, sized off the same `ServiceIconSize` scale
 * as `<ServiceIcon>`. Brand-mapped platforms render their glyph; the
 * others (`cursor`, `generic`, or anything unknown) get a Lucide fallback.
 * A null / absent / "none" platform renders nothing.
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
  const Fallback = platform === "cursor" ? MousePointer2 : Bot;
  return (
    <Fallback
      className={cn(
        LUCIDE_SIZE[size],
        "shrink-0 text-muted-foreground",
        className,
      )}
    />
  );
}

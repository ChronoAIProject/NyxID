// Lark Bot catalog tile: shared LarkFamilyGlyph + Lucide `Bot` badge.
// Standardized with api-feishu-bot.tsx — same rendering (plain glyph +
// composite badge), no chip wrapper. Same-brand plain-vs-chipped
// differentiation was dropped in favor of consistent presentation across
// all four Lark-family variants; the tile's name label carries the
// region + bot-vs-plain distinction.
import { Bot } from "lucide-react";
import { CompositeBadgeWrapper, LarkFamilyGlyph } from "./_shared";

export default function ApiLarkBotIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Bot className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <LarkFamilyGlyph data-slug="api-lark-bot" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

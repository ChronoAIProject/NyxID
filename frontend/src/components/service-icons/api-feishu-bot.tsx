// Feishu Bot catalog tile: shared Lark/Feishu glyph + Lucide `Bot` badge.
// Plain (non-chip) rendering — the chip treatment is reserved for the Lark
// variant so Feishu stays the visual "default" of the pair.
import { Bot } from "lucide-react";
import { CompositeBadgeWrapper, LarkFamilyGlyph } from "./_shared";

export default function ApiFeishuBotIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Bot className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <LarkFamilyGlyph data-slug="api-feishu-bot" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

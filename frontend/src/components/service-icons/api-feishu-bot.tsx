// Feishu Bot catalog tile: outline LarkFamilyGlyph + Lucide `Bot` badge.
// Outline matches the plain Feishu tile (see api-feishu.tsx) and pairs
// with Lark Bot's solid variant to keep the two regional bot tiles
// distinguishable at a glance.
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
      <LarkFamilyGlyph
        variant="outline"
        data-slug="api-feishu-bot"
        className="h-5 w-5"
      />
    </CompositeBadgeWrapper>
  );
}

// Lark Bot catalog tile: solid LarkFamilyGlyph + Lucide `Bot` badge.
// Solid matches the plain Lark tile (see api-lark.tsx); the outline
// variant is reserved for Feishu / Feishu Bot so the two regional
// variants stay distinguishable at a glance.
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
      <LarkFamilyGlyph
        variant="solid"
        data-slug="api-lark-bot"
        className="h-5 w-5"
      />
    </CompositeBadgeWrapper>
  );
}

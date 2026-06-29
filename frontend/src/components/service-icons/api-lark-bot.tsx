// Lark Bot catalog tile: filled, color-inverted Lark chip + Lucide `Bot`
// badge. The Lark chip differentiates Lark from Feishu (see `api-lark.tsx`);
// the badge differentiates the Bot-API variant from the plain Lark service.
// Both icons render in `currentColor` (inherited as `text-muted-foreground`
// from the tile, then overridden to `text-background` inside the Lark chip);
// no accent color — visual separation is structural (chip + badge backdrop).
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
      <span className="inline-flex h-5 w-5 items-center justify-center rounded-md bg-foreground text-background">
        <LarkFamilyGlyph data-slug="api-lark-bot" className="h-3.5 w-3.5" />
      </span>
    </CompositeBadgeWrapper>
  );
}

// Lark Bot catalog tile: solid LarkFamilyGlyph + Lucide `Bot` badge.
// Solid matches the plain Lark tile (see api-lark.tsx); the outline
// variant is reserved for Feishu / Feishu Bot so the two regional
// variants stay distinguishable at a glance.
import { Bot } from "lucide-react-native";
import { CompositeBadgeWrapper, LarkFamilyGlyph } from "./_shared";

export default function ApiLarkBotIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={<Bot strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <LarkFamilyGlyph
        variant="solid"
        data-slug="api-lark-bot"
        size={size}
        color={color}
      />
    </CompositeBadgeWrapper>
  );
}

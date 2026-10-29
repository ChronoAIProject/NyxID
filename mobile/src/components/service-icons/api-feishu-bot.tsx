// Feishu Bot catalog tile: outline LarkFamilyGlyph + Lucide `Bot` badge.
// Outline matches the plain Feishu tile (see api-feishu.tsx) and pairs
// with Lark Bot's solid variant to keep the two regional bot tiles
// distinguishable at a glance.
import { Bot } from "lucide-react-native";
import { CompositeBadgeWrapper, LarkFamilyGlyph } from "./_shared";

export default function ApiFeishuBotIcon({
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
        variant="outline"
        data-slug="api-feishu-bot"
        size={size}
        color={color}
      />
    </CompositeBadgeWrapper>
  );
}

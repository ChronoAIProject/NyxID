// Slack Bot catalog tile: Slack hash + Lucide `Bot` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Bot } from "lucide-react-native";
import { CompositeBadgeWrapper, SlackGlyph } from "./_shared";

export default function ApiSlackBotIcon({
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
      <SlackGlyph data-slug="api-slack-bot" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

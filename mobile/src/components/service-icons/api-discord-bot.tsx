// Discord Bot catalog tile: Discord mark + Lucide `Bot` badge. Both icons
// render in `currentColor` (inherited as `text-muted-foreground` from the
// tile); visual separation between the brand glyph and the badge comes from
// the badge's `bg-muted` backdrop + `ring-2 ring-background` outline in the
// shared `CompositeBadgeWrapper`.
import { Bot } from "lucide-react-native";
import { CompositeBadgeWrapper, DiscordGlyph } from "./_shared";

export default function ApiDiscordBotIcon({
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
      <DiscordGlyph data-slug="api-discord-bot" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

// Telegram Bot catalog tile: Telegram paper-plane + Lucide `Bot` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Bot } from "lucide-react-native";
import { CompositeBadgeWrapper, TelegramGlyph } from "./_shared";

export default function ApiTelegramBotIcon({
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
      <TelegramGlyph data-slug="api-telegram-bot" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

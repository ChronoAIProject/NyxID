// Telegram Bot catalog tile: Telegram paper-plane + Lucide `Bot` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Bot } from "lucide-react";
import { CompositeBadgeWrapper, TelegramGlyph } from "./_shared";

export default function ApiTelegramBotIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Bot className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <TelegramGlyph data-slug="api-telegram-bot" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

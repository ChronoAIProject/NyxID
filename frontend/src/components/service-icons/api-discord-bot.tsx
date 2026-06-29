// Discord Bot catalog tile: Discord mark + Lucide `Bot` badge. Both icons
// render in `currentColor` (inherited as `text-muted-foreground` from the
// tile); visual separation between the brand glyph and the badge comes from
// the badge's `bg-muted` backdrop + `ring-2 ring-background` outline in the
// shared `CompositeBadgeWrapper`.
import { Bot } from "lucide-react";
import { CompositeBadgeWrapper, DiscordGlyph } from "./_shared";

export default function ApiDiscordBotIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Bot className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <DiscordGlyph data-slug="api-discord-bot" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

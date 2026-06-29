// Slack Bot catalog tile: Slack hash + Lucide `Bot` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Bot } from "lucide-react";
import { CompositeBadgeWrapper, SlackGlyph } from "./_shared";

export default function ApiSlackBotIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Bot className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <SlackGlyph data-slug="api-slack-bot" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

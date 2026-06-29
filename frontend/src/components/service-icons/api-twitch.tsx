// Pure brand glyph for the Twitch catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TwitchGlyph } from "./_shared";

export default function ApiTwitchIcon({
  className,
}: {
  className?: string;
}) {
  return <TwitchGlyph data-slug="api-twitch" className={className} />;
}

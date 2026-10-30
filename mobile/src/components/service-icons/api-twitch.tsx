// Pure brand glyph for the Twitch catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TwitchGlyph } from "./_shared";

export default function ApiTwitchIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <TwitchGlyph data-slug="api-twitch" size={size} color={color} />;
}

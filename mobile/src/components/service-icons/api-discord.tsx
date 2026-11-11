// Pure brand glyph for the Discord catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { DiscordGlyph } from "./_shared";

export default function ApiDiscordIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <DiscordGlyph data-slug="api-discord" size={size} color={color} />;
}

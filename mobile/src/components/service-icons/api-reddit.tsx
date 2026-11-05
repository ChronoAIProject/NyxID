// Pure brand glyph for the Reddit catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { RedditGlyph } from "./_shared";

export default function ApiRedditIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <RedditGlyph data-slug="api-reddit" size={size} color={color} />;
}

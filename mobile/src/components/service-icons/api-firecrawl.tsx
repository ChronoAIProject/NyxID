// Pure brand glyph for the Firecrawl catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { FirecrawlGlyph } from "./_shared";

export default function ApiFirecrawlIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <FirecrawlGlyph data-slug="api-firecrawl" size={size} color={color} />;
}

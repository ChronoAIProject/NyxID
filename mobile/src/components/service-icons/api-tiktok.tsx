// Pure brand glyph for the TikTok catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TiktokGlyph } from "./_shared";

export default function ApiTiktokIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <TiktokGlyph data-slug="api-tiktok" size={size} color={color} />;
}

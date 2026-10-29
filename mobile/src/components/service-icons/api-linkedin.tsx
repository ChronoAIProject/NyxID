// Pure brand glyph for the LinkedIn catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { LinkedinGlyph } from "./_shared";

export default function ApiLinkedinIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <LinkedinGlyph data-slug="api-linkedin" size={size} color={color} />;
}

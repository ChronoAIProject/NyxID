// Pure brand glyph for the Microsoft catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { Microsoft365Glyph } from "./_shared";

export default function ApiMicrosoftIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <Microsoft365Glyph data-slug="api-microsoft" size={size} color={color} />
  );
}

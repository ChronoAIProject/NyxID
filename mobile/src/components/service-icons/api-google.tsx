// Pure brand glyph for the Google catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { GoogleGlyph } from "./_shared";

export default function ApiGoogleIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <GoogleGlyph data-slug="api-google" size={size} color={color} />;
}

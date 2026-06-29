// Pure brand glyph for the Google catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { GoogleGlyph } from "./_shared";

export default function ApiGoogleIcon({
  className,
}: {
  className?: string;
}) {
  return <GoogleGlyph data-slug="api-google" className={className} />;
}

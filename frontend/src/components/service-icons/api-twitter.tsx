// Pure brand glyph for the Twitter / X catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TwitterGlyph } from "./_shared";

export default function ApiTwitterIcon({
  className,
}: {
  className?: string;
}) {
  return <TwitterGlyph data-slug="api-twitter" className={className} />;
}

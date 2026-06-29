// Pure brand glyph for the Facebook catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { FacebookGlyph } from "./_shared";

export default function ApiFacebookIcon({
  className,
}: {
  className?: string;
}) {
  return <FacebookGlyph data-slug="api-facebook" className={className} />;
}

// Pure brand glyph for the LinkedIn catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { LinkedinGlyph } from "./_shared";

export default function ApiLinkedinIcon({
  className,
}: {
  className?: string;
}) {
  return <LinkedinGlyph data-slug="api-linkedin" className={className} />;
}

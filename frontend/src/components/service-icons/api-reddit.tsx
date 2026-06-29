// Pure brand glyph for the Reddit catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { RedditGlyph } from "./_shared";

export default function ApiRedditIcon({
  className,
}: {
  className?: string;
}) {
  return <RedditGlyph data-slug="api-reddit" className={className} />;
}

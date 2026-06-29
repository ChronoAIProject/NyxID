// Pure brand glyph for the TikTok catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TiktokGlyph } from "./_shared";

export default function ApiTiktokIcon({
  className,
}: {
  className?: string;
}) {
  return <TiktokGlyph data-slug="api-tiktok" className={className} />;
}

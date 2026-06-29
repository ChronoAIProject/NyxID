// Pure brand glyph for the Discord catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { DiscordGlyph } from "./_shared";

export default function ApiDiscordIcon({
  className,
}: {
  className?: string;
}) {
  return <DiscordGlyph data-slug="api-discord" className={className} />;
}

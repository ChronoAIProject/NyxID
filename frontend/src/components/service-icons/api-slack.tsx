// Pure brand glyph for the Slack catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { SlackGlyph } from "./_shared";

export default function ApiSlackIcon({
  className,
}: {
  className?: string;
}) {
  return <SlackGlyph data-slug="api-slack" className={className} />;
}

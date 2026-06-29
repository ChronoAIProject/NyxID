// Feishu catalog tile. Shares the `LarkFamilyGlyph` with Lark (same parent
// brand); Feishu renders the glyph plain (`currentColor` on transparent),
// while Lark wraps the same glyph in a filled, color-inverted chip — that's
// the only visual difference between the two tiles.
import { LarkFamilyGlyph } from "./_shared";

export default function ApiFeishuIcon({
  className,
}: {
  className?: string;
}) {
  return <LarkFamilyGlyph data-slug="api-feishu" className={className} />;
}

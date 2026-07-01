// Lark catalog tile. Renders the shared LarkFamilyGlyph plain — no chip
// wrapper. Lark and Feishu are the same product marketed under different
// names in different regions; the icon differentiation via a filled chip
// was dropped so all four Lark-family tiles (Lark, Lark Bot, Feishu,
// Feishu Bot) render identically at the glyph layer, matching how every
// other family in the catalog handles cross-region variants. The tile's
// name label carries the region distinction.
import { LarkFamilyGlyph } from "./_shared";

export default function ApiLarkIcon({ className }: { className?: string }) {
  return <LarkFamilyGlyph data-slug="api-lark" className={className} />;
}

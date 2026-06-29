// Lark catalog tile. Lark and Feishu share the same parent brand mark, so the
// shared `LarkFamilyGlyph` is reused here — visual differentiation comes from
// rendering Lark inside a filled, color-inverted rounded chip (`bg-foreground`
// + `text-background`) while Feishu renders plain `currentColor` on
// transparent. This is intentional per design feedback: same shape, distinct
// surface treatment, no second-color accent on either.
import { LarkFamilyGlyph } from "./_shared";

export default function ApiLarkIcon({ className }: { className?: string }) {
  return (
    <span
      className={`inline-flex h-5 w-5 items-center justify-center rounded-md bg-foreground text-background ${className ?? ""}`}
    >
      <LarkFamilyGlyph data-slug="api-lark" className="h-3.5 w-3.5" />
    </span>
  );
}

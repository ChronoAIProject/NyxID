// Pure brand glyph for the Mistral AI catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { MistralGlyph } from "./_shared";

export default function LlmMistralIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <MistralGlyph data-slug="llm-mistral" size={size} color={color} />;
}

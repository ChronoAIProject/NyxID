// Pure brand glyph for the Cohere catalog tile. Hand-built stylized "C" mark.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { CohereGlyph } from "./_shared";

export default function LlmCohereIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <CohereGlyph data-slug="llm-cohere" size={size} color={color} />;
}

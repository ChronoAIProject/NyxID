// Pure brand glyph for the OpenAI catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { OpenAiGlyph } from "./_shared";

export default function LlmOpenaiIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <OpenAiGlyph data-slug="llm-openai" size={size} color={color} />;
}

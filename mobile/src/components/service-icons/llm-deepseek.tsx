// Pure brand glyph for the DeepSeek catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { DeepSeekGlyph } from "./_shared";

export default function LlmDeepSeekIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <DeepSeekGlyph data-slug="llm-deepseek" size={size} color={color} />;
}

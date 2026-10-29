// OpenAI Codex catalog tile: OpenAI knot + Lucide `Code` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Code } from "lucide-react-native";
import { CompositeBadgeWrapper, OpenAiGlyph } from "./_shared";

export default function LlmOpenaiCodexIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={<Code strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <OpenAiGlyph data-slug="llm-openai-codex" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

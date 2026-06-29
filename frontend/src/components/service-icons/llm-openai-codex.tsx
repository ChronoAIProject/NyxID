// OpenAI Codex catalog tile: OpenAI knot + Lucide `Code` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Code } from "lucide-react";
import { CompositeBadgeWrapper, OpenAiGlyph } from "./_shared";

export default function LlmOpenaiCodexIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Code className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <OpenAiGlyph data-slug="llm-openai-codex" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

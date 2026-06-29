// Pure brand glyph for the Mistral AI catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { MistralGlyph } from "./_shared";

export default function LlmMistralIcon({ className }: { className?: string }) {
  return <MistralGlyph data-slug="llm-mistral" className={className} />;
}

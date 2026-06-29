// Pure brand glyph for the Cohere catalog tile. Hand-built stylized "C" mark.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { CohereGlyph } from "./_shared";

export default function LlmCohereIcon({ className }: { className?: string }) {
  return <CohereGlyph data-slug="llm-cohere" className={className} />;
}

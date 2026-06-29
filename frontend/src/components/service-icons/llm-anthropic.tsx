// Pure brand glyph for the Anthropic catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { AnthropicGlyph } from "./_shared";

export default function LlmAnthropicIcon({ className }: { className?: string }) {
  return <AnthropicGlyph data-slug="llm-anthropic" className={className} />;
}

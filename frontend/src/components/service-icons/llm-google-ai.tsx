// Pure brand glyph for the Google AI Studio catalog tile. Reuses the
// monochrome Google "G" mark (Google AI Studio / Gemini has no dedicated
// Simple Icons entry — the household "G" reads clearly at 20px).
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { GoogleGlyph } from "./_shared";

export default function LlmGoogleAiIcon({
  className,
}: {
  className?: string;
}) {
  return <GoogleGlyph data-slug="llm-google-ai" className={className} />;
}

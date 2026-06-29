// Google AI Studio catalog tile: Google G + Lucide `Sparkles` badge. Per
// design feedback, services from the same platform must use distinct
// composite badges — this tile shares the Google "G" brand glyph with
// `api-google` (plain) and `api-google-cloud` (Cloud badge), and the
// `Sparkles` badge here marks it as the Gemini / AI Studio variant.
import { Sparkles } from "lucide-react";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function LlmGoogleAiIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Sparkles className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <GoogleGlyph data-slug="llm-google-ai" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

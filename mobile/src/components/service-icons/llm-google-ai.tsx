// Google AI Studio catalog tile: Google G + Lucide `Sparkles` badge. Per
// design feedback, services from the same platform must use distinct
// composite badges — this tile shares the Google "G" brand glyph with
// `api-google` (plain) and `api-google-cloud` (Cloud badge), and the
// `Sparkles` badge here marks it as the Gemini / AI Studio variant.
import { Sparkles } from "lucide-react-native";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function LlmGoogleAiIcon({
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
      badge={<Sparkles strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <GoogleGlyph data-slug="llm-google-ai" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

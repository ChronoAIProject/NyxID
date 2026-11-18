// Google Cloud catalog tile: Google G + Lucide `Cloud` badge. The Cloud
// badge differentiates this tile from the plain `api-google` (OAuth/Google
// account) and `llm-google-ai` (Google AI Studio) tiles, which share the same
// "G" brand mark but use different badge glyphs.
import { Cloud } from "lucide-react-native";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleCloudIcon({
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
      badge={<Cloud strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <GoogleGlyph data-slug="api-google-cloud" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

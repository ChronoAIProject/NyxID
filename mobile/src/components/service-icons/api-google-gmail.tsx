import { Mail } from "lucide-react-native";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleGmailIcon({
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
      badge={<Mail strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <GoogleGlyph data-slug="api-google-gmail" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

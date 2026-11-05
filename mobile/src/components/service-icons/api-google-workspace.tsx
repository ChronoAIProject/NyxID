import { BriefcaseBusiness } from "lucide-react-native";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleWorkspaceIcon({
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
      badge={
        <BriefcaseBusiness
          strokeWidth={2.5}
          size={size * 0.7 - 2}
          color={color}
        />
      }
    >
      <GoogleGlyph data-slug="api-google-workspace" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

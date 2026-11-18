import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
// IFTTT platform wordmark, rendered in the catalog tile's color.
export default function ApiIftttIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <GlyphSvg
      viewBox="0 0 203 52"
      fill="none"
      data-slug="api-ifttt"
      size={size}
      color={color}
    >
      <Path
        d="M109.374-.25H68.0791V15.3654H80.3558V52.1734H97.0968V15.3654H109.374V-.25ZM156.249-.25H114.954V15.3654H127.231V52.1734H143.972V15.3654H156.249V-.25ZM203.123-.25H161.829V15.3654H174.105V52.1734H190.846V15.3654H203.123V-.25ZM16.741-.25H0V52.1734H16.741V-.25ZM62.4997-.25H24.5535V52.1734H41.2945V37.6734H55.8033V20.9425H41.2945V15.3654H62.4997V-.25Z"
        fill="currentColor"
      />
    </GlyphSvg>
  );
}

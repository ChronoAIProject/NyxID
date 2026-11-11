import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
// Asana mark from Simple Icons (CC0), in the catalog's monochrome style.
export default function ApiAsanaIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <GlyphSvg
      viewBox="0 0 24 24"
      fill="currentColor"
      data-slug="api-asana"
      size={size}
      color={color}
    >
      <Path d="M18.78 12.653c-2.882 0-5.22 2.336-5.22 5.22s2.338 5.22 5.22 5.22 5.22-2.34 5.22-5.22-2.336-5.22-5.22-5.22zm-13.56 0c-2.88 0-5.22 2.337-5.22 5.22s2.338 5.22 5.22 5.22 5.22-2.338 5.22-5.22-2.336-5.22-5.22-5.22zm12-6.525c0 2.883-2.337 5.22-5.22 5.22-2.882 0-5.22-2.337-5.22-5.22 0-2.88 2.338-5.22 5.22-5.22 2.883 0 5.22 2.34 5.22 5.22z" />
    </GlyphSvg>
  );
}

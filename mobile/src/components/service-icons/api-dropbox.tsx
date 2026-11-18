import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
// Dropbox mark from Simple Icons (CC0), in the catalog's monochrome style.
export default function ApiDropboxIcon({
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
      data-slug="api-dropbox"
      size={size}
      color={color}
    >
      <Path d="M6 1.807L0 5.629l6 3.822 6.001-3.822L6 1.807zM18 1.807l-6 3.822 6 3.822 6-3.822-6-3.822zM0 13.274l6 3.822 6.001-3.822L6 9.452l-6 3.822zM18 9.452l-6 3.822 6 3.822 6-3.822-6-3.822zM6 18.371l6.001 3.822 6-3.822-6-3.822L6 18.371z" />
    </GlyphSvg>
  );
}

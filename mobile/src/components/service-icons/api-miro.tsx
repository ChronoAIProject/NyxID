import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
// Miro mark from Simple Icons (CC0), in the catalog's monochrome style.
export default function ApiMiroIcon({
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
      data-slug="api-miro"
      size={size}
      color={color}
    >
      <Path d="M17.392 0H13.9L17 4.808 10.444 0H6.949l3.102 6.3L3.494 0H0l3.05 8.131L0 24h3.494L10.05 6.985 6.949 24h3.494L17 5.494 13.899 24h3.493L24 3.672 17.392 0z" />
    </GlyphSvg>
  );
}

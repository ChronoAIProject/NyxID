import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
// Productboard mark from SVG Logos (gilbarbara, CC0). Its three colour
// blocks are separated by a small gap so they stay distinct in the catalog's
// monochrome style.
export default function ApiProductboardIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <GlyphSvg
      viewBox="0 -44 256 256"
      fill="currentColor"
      data-slug="api-productboard"
      size={size}
      color={color}
    >
      <Path d="M85.33 89.61 L160.89 163.99 L9.77 163.99z" />
      <Path d="M9.77 4 L85.33 78.38 L160.89 4z" />
      <Path d="M91.04 84 L170.67 162.38 L250.29 84 L170.67 5.61z" />
    </GlyphSvg>
  );
}

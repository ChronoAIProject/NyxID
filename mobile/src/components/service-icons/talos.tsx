import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
import type { ServiceIconProps } from "./index";

export default function TalosIcon({ size = 20, color }: ServiceIconProps) {
  return (
    <GlyphSvg
      viewBox="0 0 424 424"
      size={size}
      color={color}
      fill="currentColor"
      data-slug="talos"
    >
      <Path d="M1 1H336C384 1 423 40 423 88V141H71C32 141 1 110 1 71Z" />
      <Path d="M142 165H282V353C282 392 251 423 212 423H142Z" />
    </GlyphSvg>
  );
}

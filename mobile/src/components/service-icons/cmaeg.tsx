import { Path } from "react-native-svg";
import { GlyphSvg } from "./_shared";
import type { ServiceIconProps } from "./index";

export default function AgentEventGatewayIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return (
    <GlyphSvg
      viewBox="0 0 64 64"
      size={size}
      color={color}
      fill="currentColor"
      data-slug="cmaeg"
    >
      <Path d="M6 60V27L19 9L30 3.9V19L19 31V60Z" />
      <Path d="M58 60V27L45 9L34 3.9V19L45 31V60Z" />
      <Path d="M32 29L39 38V49L32 58L25 49V38Z" />
    </GlyphSvg>
  );
}

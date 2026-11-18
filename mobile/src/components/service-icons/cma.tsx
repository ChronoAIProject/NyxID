import { Path, Rect } from "react-native-svg";
import { GlyphSvg } from "./_shared";
import type { GlyphProps } from "./_shared";
import type { ServiceIconProps } from "./index";

export default function CmaIcon({ size = 20, color }: ServiceIconProps) {
  return <CmaGlyph size={size} color={color} data-slug="cma" />;
}

/** Static CMA mark. The supplied animated asset is intentionally rendered as
 * a still icon so service lists do not shimmer while users scan them. */
export function CmaGlyph(props: GlyphProps & { "data-slug"?: string }) {
  return (
    <GlyphSvg viewBox="13 13 74 65" data-cma-glyph="true" {...props}>
      <Path
        d="M71 71 A30 30 0 1 0 29 71"
        fill="none"
        stroke="currentColor"
        strokeWidth="11"
        strokeLinecap="round"
      />
      <Path
        d="M71 71 L61 59 L50 71 L39 59 L29 71"
        fill="none"
        stroke="currentColor"
        strokeWidth="10"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <Rect x="30" y="38" width="17" height="12" rx="4" fill="currentColor" />
      <Rect x="53" y="38" width="17" height="12" rx="4" fill="currentColor" />
      <Path
        d="M46 44 L54 44"
        fill="none"
        stroke="currentColor"
        strokeWidth="4"
      />
    </GlyphSvg>
  );
}

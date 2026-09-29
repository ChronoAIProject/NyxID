import type { SVGProps } from "react";
import type { ServiceIconProps } from "./index";

export default function CmaIcon({ className }: ServiceIconProps) {
  return <CmaGlyph className={className} data-slug="cma" />;
}

/** Static CMA mark. The supplied animated asset is intentionally rendered as
 * a still icon so service lists do not shimmer while users scan them. */
export function CmaGlyph(
  props: SVGProps<SVGSVGElement> & { "data-slug"?: string },
) {
  return (
    <svg
      viewBox="13 13 74 65"
      aria-hidden="true"
      data-cma-glyph="true"
      {...props}
    >
      <path
        d="M71 71 A30 30 0 1 0 29 71"
        fill="none"
        stroke="currentColor"
        strokeWidth="11"
        strokeLinecap="round"
      />
      <path
        d="M71 71 L61 59 L50 71 L39 59 L29 71"
        fill="none"
        stroke="currentColor"
        strokeWidth="10"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <rect x="30" y="38" width="17" height="12" rx="4" fill="currentColor" />
      <rect x="53" y="38" width="17" height="12" rx="4" fill="currentColor" />
      <path
        d="M46 44 L54 44"
        fill="none"
        stroke="currentColor"
        strokeWidth="4"
      />
    </svg>
  );
}

import { useId } from "react";

/**
 * Portal Mark — pixel-accurate replica of VoidPortal.pen:
 * radial glow, three gradient-stroke arc ellipses (partial arcs),
 * Void Moon crescent path, four particle dots.
 *
 * Uses `gradientUnits="userSpaceOnUse"` with correct rotation
 * centers to match Pencil's rendering of gradient strokes.
 */
export interface PortalMarkLogoProps {
  readonly className?: string;
  readonly size?: number;
}

export function PortalMarkLogo({
  className,
  size = 32,
}: PortalMarkLogoProps) {
  const uid = useId().replace(/:/g, "");

  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 130 130"
      fill="none"
      aria-hidden
    >
      <defs>
        {/* ── Glow ── */}
        <radialGradient id={`${uid}g`} cx="65" cy="65" r="65" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#8B5CF6" stopOpacity="0.08" />
          <stop offset="100%" stopColor="#8B5CF6" stopOpacity="0" />
        </radialGradient>

        {/* ── Arc Outer — rotation 0° (left → right) ── */}
        <linearGradient
          id={`${uid}ao`}
          gradientUnits="userSpaceOnUse"
          x1="10" y1="65" x2="120" y2="65"
        >
          <stop offset="0" stopColor="#A78BFA" />
          <stop offset="0.5" stopColor="#A78BFA" stopOpacity="0" />
        </linearGradient>

        {/* ── Arc Mid — rotation 120° ── */}
        <linearGradient
          id={`${uid}am`}
          gradientUnits="userSpaceOnUse"
          x1="10" y1="65" x2="120" y2="65"
          gradientTransform="rotate(120 65 65)"
        >
          <stop offset="0" stopColor="#C4B5FD" />
          <stop offset="0.5" stopColor="#C4B5FD" stopOpacity="0" />
        </linearGradient>

        {/* ── Arc Inner — rotation 240° ── */}
        <linearGradient
          id={`${uid}ai`}
          gradientUnits="userSpaceOnUse"
          x1="10" y1="65" x2="120" y2="65"
          gradientTransform="rotate(240 65 65)"
        >
          <stop offset="0" stopColor="#DDD6FE" />
          <stop offset="0.5" stopColor="#DDD6FE" stopOpacity="0" />
        </linearGradient>

        {/* ── Void Moon fill — rotation 160° ── */}
        <linearGradient
          id={`${uid}vm`}
          gradientUnits="userSpaceOnUse"
          x1="56" y1="62" x2="86" y2="62"
          gradientTransform="rotate(160 71 62)"
        >
          <stop offset="0" stopColor="#C4B5FD" />
          <stop offset="1" stopColor="#7C3AED" />
        </linearGradient>
      </defs>

      {/* Radial glow background */}
      <circle cx="65" cy="65" r="65" fill={`url(#${uid}g)`} />

      {/* Arc Outer: 110×110 at (10,10) → c(65,65) r=55 */}
      <circle cx="65" cy="65" r="55" fill="none" stroke={`url(#${uid}ao)`} strokeWidth="1" />

      {/* Arc Mid: 80×80 at (25,25) → c(65,65) r=40 */}
      <circle cx="65" cy="65" r="40" fill="none" stroke={`url(#${uid}am)`} strokeWidth="1" />

      {/* Arc Inner: 50×50 at (40,40) → c(65,65) r=25 */}
      <circle cx="65" cy="65" r="25" fill="none" stroke={`url(#${uid}ai)`} strokeWidth="0.8" />

      {/* Void Moon crescent at (56, 42) */}
      <path
        d="M24 0q6 8 6 20 0 12-6 20-14-4-20-12-4-14-2-24 4-4 22-4z"
        transform="translate(56 42)"
        fill={`url(#${uid}vm)`}
      />

      {/* Particles */}
      <circle cx="31.5" cy="49.5" r="1.5" fill="#C4B5FD" />
      <circle cx="39" cy="63" r="1" fill="#C4B5FD" opacity="0.5" />
      <circle cx="25" cy="69" r="1" fill="#C4B5FD" opacity="0.31" />
      <circle cx="89" cy="25" r="1" fill="#A78BFA" opacity="0.25" />
    </svg>
  );
}

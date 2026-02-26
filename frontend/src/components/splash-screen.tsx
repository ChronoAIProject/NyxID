import { useEffect, useState } from "react";

interface SplashScreenProps {
  readonly onFinish: () => void;
  readonly minDuration?: number;
}

export function SplashScreen({ onFinish, minDuration = 1800 }: SplashScreenProps) {
  const [fadeOut, setFadeOut] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => setFadeOut(true), minDuration);
    return () => clearTimeout(timer);
  }, [minDuration]);

  useEffect(() => {
    if (!fadeOut) return;
    const timer = setTimeout(onFinish, 500);
    return () => clearTimeout(timer);
  }, [fadeOut, onFinish]);

  return (
    <div
      className={`fixed inset-0 z-[9999] flex flex-col items-center justify-center gap-10 transition-opacity duration-500 ${fadeOut ? "opacity-0" : "opacity-100"}`}
      style={{ backgroundColor: "#06060A" }}
    >
      {/* ── Portal Mark (SVG) ── */}
      <svg
        width="140"
        height="140"
        viewBox="0 0 280 280"
        fill="none"
        className="animate-[splash-pulse_2.4s_ease-in-out_infinite]"
      >
        <defs>
          <radialGradient id="sp-glow" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#8B5CF6" stopOpacity="0.12" />
            <stop offset="100%" stopColor="#8B5CF6" stopOpacity="0" />
          </radialGradient>
          <linearGradient id="sp-arc1" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#A78BFA" />
            <stop offset="50%" stopColor="#A78BFA" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="sp-arc2" x1="0.93" y1="0" x2="0.07" y2="1">
            <stop offset="0%" stopColor="#C4B5FD" />
            <stop offset="50%" stopColor="#C4B5FD" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="sp-arc3" x1="0.07" y1="1" x2="0.93" y2="0">
            <stop offset="0%" stopColor="#DDD6FE" />
            <stop offset="50%" stopColor="#DDD6FE" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="sp-moon" x1="0.5" y1="0" x2="0.3" y2="1">
            <stop offset="0%" stopColor="#C4B5FD" />
            <stop offset="100%" stopColor="#7C3AED" />
          </linearGradient>
        </defs>
        <ellipse cx="140" cy="140" rx="140" ry="140" fill="url(#sp-glow)" />
        <ellipse cx="140" cy="140" rx="118" ry="118" stroke="url(#sp-arc1)" strokeWidth="1.6" />
        <ellipse cx="140" cy="140" rx="86" ry="86" stroke="url(#sp-arc2)" strokeWidth="1.4" />
        <ellipse cx="140" cy="140" rx="54" ry="54" stroke="url(#sp-arc3)" strokeWidth="1.1" />
        <g transform="translate(121, 90) scale(1.077)">
          <path d="M24 0q6 8 6 20 0 12-6 20-14-4-20-12-4-14-2-24 4-4 22-4z" fill="url(#sp-moon)" />
        </g>
        <circle cx="65" cy="104" r="3" fill="#C4B5FD" />
        <circle cx="83" cy="135" r="2.5" fill="#C4B5FD" opacity="0.5" />
        <circle cx="55" cy="148" r="2" fill="#C4B5FD" opacity="0.31" />
        <circle cx="191" cy="55" r="2" fill="#A78BFA" opacity="0.25" />
      </svg>

      {/* ── Brand text ── */}
      <div
        className="flex items-baseline gap-0.5 animate-[splash-fade-up_0.8s_ease-out_0.3s_both]"
      >
        <span className="font-logo text-[32px] font-normal text-void-50">N</span>
        <span className="font-logo text-[32px] font-normal text-void-200">yx</span>
        <span className="font-sans text-[32px] font-light tracking-wider text-void-300">ID</span>
      </div>

      {/* ── Tagline ── */}
      <p
        className="text-[10px] font-medium tracking-[0.35em] text-void-400/60 uppercase animate-[splash-fade-up_0.8s_ease-out_0.6s_both]"
      >
        Guardian of Digital Identity
      </p>
    </div>
  );
}

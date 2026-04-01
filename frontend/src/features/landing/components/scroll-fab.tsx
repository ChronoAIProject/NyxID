import { useState, useCallback } from "react";
import { useScroll } from "../hooks/use-scroll";

export function ScrollFab() {
  const [progress, setProgress] = useState(0);
  const [visible, setVisible] = useState(false);

  useScroll((scrollY, vh) => {
    setVisible(scrollY > vh);
    const docHeight = document.documentElement.scrollHeight - vh;
    const fillStart = vh;
    const p =
      docHeight > fillStart
        ? Math.max(
            0,
            Math.min(1, (scrollY - fillStart) / (docHeight - fillStart)),
          )
        : 0;
    setProgress(p);
  });

  const scrollToTop = useCallback(() => {
    window.scrollTo({ top: 0, behavior: "smooth" });
  }, []);

  const size = 48;
  const maskCx = 26 + progress * 24;

  return (
    <button
      onClick={scrollToTop}
      className={`fixed z-50 flex items-center justify-center rounded-full border bg-landing-surface/90 backdrop-blur-sm transition-all duration-500 ${
        progress > 0.95
          ? "border-primary/50 fab-glow-full"
          : "border-primary/30 shadow-lg shadow-primary/20"
      } hover:border-primary/50 hover:fab-glow-hover`}
      style={{
        bottom: 24,
        right: 24,
        width: size,
        height: size,
        opacity: visible ? 1 : 0,
        transform: visible ? "translateY(0)" : "translateY(20px)",
        pointerEvents: visible ? "auto" : "none",
      }}
      aria-label="Scroll to top"
    >
      <svg
        width={size}
        height={size}
        viewBox="0 0 44 44"
        className="absolute inset-0"
        style={{ zIndex: 0 }}
      >
        <defs>
          <mask id="fab-moon-mask">
            <rect width="44" height="44" fill="white" />
            <circle cx={maskCx} cy="22" r="14" fill="black" />
          </mask>
        </defs>
        <circle
          cx="22"
          cy="22"
          r="15"
          fill="#8B5CF6"
          opacity={0.5}
          mask="url(#fab-moon-mask)"
        />
      </svg>

      <svg
        width={20}
        height={22}
        viewBox="0 0 20 22"
        fill="none"
        stroke="#DDD6FE"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        className="relative"
        style={{ zIndex: 10 }}
      >
        <polyline points="4,10 10,4 16,10" />
        <polyline points="4,18 10,12 16,18" />
      </svg>
    </button>
  );
}

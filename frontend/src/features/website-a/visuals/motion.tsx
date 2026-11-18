import { createContext, useContext, useEffect, useState } from "react";
export const MotionContext = createContext("static");
export function useVisualMotion() {
  const mode = useContext(MotionContext);
  return { mode, allowed: mode !== "static", foreground: mode === "active" };
}
export function useMotionTarget(target: HTMLElement | null, threshold = 0.01) {
  const motion = useVisualMotion();
  const [viewport, setViewport] = useState({ visible: false, seen: false });
  useEffect(() => {
    if (!target) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.some((entry) => entry.isIntersecting);
        setViewport((previous) => ({
          visible,
          seen: previous.seen || visible,
        }));
      },
      { threshold },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [target, threshold]);
  return {
    ...motion,
    ...viewport,
    active: motion.allowed && motion.foreground && viewport.visible,
  };
}

"use client";

// React Bits FadeContent, pinned in PROVENANCE.md. Native target adapter:
// the shared IntersectionObserver gate replaces ScrollTrigger for presentation frames.
import { useEffect, useRef } from "react";
import type { HTMLAttributes, ReactNode } from "react";
import { gsap } from "gsap";

interface FadeContentProps extends HTMLAttributes<HTMLDivElement> {
  target?: HTMLElement;
  children?: ReactNode;
  active?: boolean;
  motionAllowed?: boolean;
  blur?: boolean;
  duration?: number;
  ease?: string;
  delay?: number;
  threshold?: number;
  initialOpacity?: number;
}

export default function FadeContent({
  children,
  target,
  active = true,
  motionAllowed = true,
  blur = false,
  duration = 1000,
  ease = "power2.out",
  delay = 0,
  initialOpacity = 0,
  threshold: _threshold,
  className = "",
  style,
  ...props
}: FadeContentProps) {
  void _threshold;
  const ref = useRef<HTMLDivElement>(null);
  const timeline = useRef<gsap.core.Timeline | null>(null);
  const completed = useRef(false);
  const isActive = useRef(active);
  useEffect(() => {
    isActive.current = active;
    if (active) timeline.current?.resume();
    else timeline.current?.pause();
  }, [active]);

  useEffect(() => {
    const el = target || ref.current;
    if (!el) return;
    if (
      !motionAllowed ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      completed.current = true;
      return;
    }
    if (completed.current) return;
    const originalStyle = el.getAttribute("style");
    let tl: gsap.core.Timeline | null = null;
    try {
      gsap.set(el, {
        autoAlpha: initialOpacity,
        ...(blur ? { filter: "blur(10px)" } : {}),
        willChange: "opacity",
      });
      tl = gsap.timeline({
        paused: true,
        delay: delay > 10 ? delay / 1000 : delay,
        onComplete: () => {
          completed.current = true;
          gsap.set(el, { clearProps: "willChange" });
        },
      });
      tl.to(el, {
        autoAlpha: 1,
        ...(blur ? { filter: "blur(0px)" } : {}),
        duration: duration > 10 ? duration / 1000 : duration,
        ease,
      });
      timeline.current = tl;
      if (isActive.current) tl.play();
    } catch {
      completed.current = true;
      console.warn(
        "[nyxid-visual-v4] Entrance unavailable; original content retained.",
      );
      if (originalStyle === null) el.removeAttribute("style");
      else el.setAttribute("style", originalStyle);
    }
    return () => {
      tl?.kill();
      gsap.killTweensOf(el);
      timeline.current = null;
      if (originalStyle === null) el.removeAttribute("style");
      else el.setAttribute("style", originalStyle);
    };
  }, [target, motionAllowed, blur, duration, delay, initialOpacity, ease]);

  if (target) return null;
  // Static HTML remains readable if JavaScript, the gate or GSAP fails.
  return (
    <div ref={ref} className={className} style={style} {...props}>
      {children}
    </div>
  );
}

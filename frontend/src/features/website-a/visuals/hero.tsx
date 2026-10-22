import { useEffect, useRef, useState } from "react";
import type { HTMLAttributes } from "react";
import GradientText from "./react-bits/GradientText";
import { useMotionTarget, useVisualMotion } from "./motion";
import { loadBackgroundRuntime } from "./background-runtime";
import type { HeroScene } from "./background-runtime";
import { visualConfig } from "./config";
import { useWebsite } from "../website-context";

export function HeroTitle() {
  const { language } = useWebsite();
  const [target, setTarget] = useState<HTMLHeadingElement | null>(null);
  const { active } = useMotionTarget(target);
  const text = language === "zh" ? "超越聊天。" : "more than chat.";
  return (
    <h1 ref={setTarget} className="h1">
      {language === "zh" ? "AI，" : "AI that does"}
      <br />
      {active ? (
        <GradientText
          animationSpeed={9}
          colors={["#A672FB", "#5E00F5"]}
          scale={2}
          className="nyx-title-gradient"
        >
          {text}
        </GradientText>
      ) : (
        <span className="nyx-title-static">{text}</span>
      )}
    </h1>
  );
}

export function HeroBackground({ target }: { target: HTMLElement | null }) {
  const { allowed, active, seen } = useMotionTarget(target);
  const container = useRef<HTMLDivElement>(null);
  const scene = useRef<HeroScene | null>(null);
  const activeRef = useRef(active);
  const [status, setStatus] = useState("static");
  useEffect(() => {
    activeRef.current = active;
    if (scene.current) scene.current.paused = !active;
  }, [active]);
  const start = allowed && seen;
  useEffect(() => {
    if (!start || !container.current) return;
    let cancelled = false;
    const element = container.current;
    let pending = true;
    const fallback = window.setTimeout(() => {
      if (pending && !cancelled) setStatus("fallback");
    }, 8000);
    const delay = window.setTimeout(() => {
      setStatus("loading");
      loadBackgroundRuntime()
        .then((runtime) => {
          if (cancelled) return;
          return runtime.addScene({
            projectId: visualConfig.projectId,
            element,
            fps: 30,
            dpi: 1,
            interactivity: { mouse: { disabled: true } },
          });
        })
        .then((result) => {
          if (!result) return;
          if (cancelled) {
            result.destroy();
            return;
          }
          scene.current = result;
          result.paused = !activeRef.current;
          setStatus("ready");
        })
        .catch(() => {
          if (!cancelled) {
            setStatus("fallback");
            console.warn(
              "[nyxid-visual-v4] Hero unavailable; static background retained.",
            );
          }
        })
        .finally(() => {
          pending = false;
          window.clearTimeout(fallback);
        });
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(delay);
      window.clearTimeout(fallback);
      scene.current?.destroy();
      scene.current = null;
    };
  }, [start]);
  return (
    <div
      ref={container}
      className="nyx-hero-scene"
      aria-hidden="true"
      inert
      data-scene-state={allowed ? status : "static"}
      data-scene-paused={!active}
    />
  );
}

export function HeroSection({
  children,
  className = "",
  ...props
}: HTMLAttributes<HTMLElement>) {
  const [target, setTarget] = useState<HTMLElement | null>(null);
  const { mode } = useVisualMotion();
  return (
    <section
      {...props}
      ref={setTarget}
      className={`${className} nyx-home-hero`}
      data-motion={mode}
    >
      <HeroBackground target={target} />
      {children}
    </section>
  );
}

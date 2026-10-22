import { SiteLink } from "../site-link";
import { useState } from "react";
import type { AnchorHTMLAttributes, HTMLAttributes } from "react";
import SpecularButton from "./react-bits/SpecularButton";
import SpotlightCard from "./react-bits/SpotlightCard";
import FadeContent from "./react-bits/FadeContent";
import { useMotionTarget } from "./motion";
import { WEBSITE_THEME } from "../website-theme";

export function VisualLink({
  effect,
  className = "",
  children,
  hidden,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement> & { effect: "button" | "card" }) {
  const [target, setTarget] = useState<HTMLAnchorElement | null>(null);
  const { active } = useMotionTarget(target);
  return (
    <SiteLink
      {...props}
      hidden={hidden}
      ref={setTarget}
      className={`${className} ${effect === "button" ? "nyx-primary" : "nyx-clickable"}`}
      data-visual-effect={effect}
    >
      {children}
      {active &&
        !hidden &&
        target &&
        (effect === "button" ? (
          <span className="nyx-button-mount" aria-hidden="true">
            <SpecularButton
              target={target}
              radius={className.split(" ").includes("sm") ? 8 : 10}
              lineColor="#E8DFFF"
              baseColor="#A672FB"
              intensity={1.2}
              idleIntensity={0.45}
              autoAnimate={false}
              followMouse
              proximity={90}
              glow={0.6}
              thickness={1.2}
            />
          </span>
        ) : (
          <SpotlightCard
            theme={WEBSITE_THEME}
            target={target}
            className="nyx-card-light"
            spotlightColor="#A672FB"
            intensity={0.1}
            borderGlow={0.5}
            ambient={false}
            flare={false}
            proximity={24}
            aria-hidden="true"
          />
        ))}
    </SiteLink>
  );
}

export function RevealFrame({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  const [target, setTarget] = useState<HTMLDivElement | null>(null);
  const { active, allowed } = useMotionTarget(target, 0.08);
  return (
    <div
      {...props}
      ref={setTarget}
      className={className}
      data-visual-reveal="once"
    >
      {children}
      {target && (
        <FadeContent
          target={target}
          active={active}
          motionAllowed={allowed}
          duration={600}
          initialOpacity={0.72}
          threshold={0.08}
          blur={false}
        />
      )}
    </div>
  );
}

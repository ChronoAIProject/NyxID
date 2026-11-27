import { Fragment, useId } from "react";
import {
  PROP_ART,
  propTransform,
  staticPropPose,
  type PropNode,
  type PropShape,
  propMaterial,
} from "../../../../../shared/agent-avatars/props";
import type {
  AvatarMood,
  AvatarConfig,
  AvatarTheme,
} from "@/lib/assistant/agent-avatar-model";

function Shape({
  shape,
  paint,
}: {
  shape: PropShape;
  paint: (colour: string) => string;
}) {
  const common = {
    fill: paint(shape.fill),
    stroke: shape.stroke ? paint(shape.stroke) : undefined,
    strokeWidth: shape.strokeWidth,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
  if (shape.type === "rect")
    return (
      <rect
        {...common}
        x={shape.x}
        y={shape.y}
        width={shape.width}
        height={shape.height}
        rx={shape.rx}
      />
    );
  if (shape.type === "circle")
    return <circle {...common} cx={shape.cx} cy={shape.cy} r={shape.r} />;
  const points = shape.points?.map((point) => point.join(",")).join(" ");
  return shape.type === "polygon" ? (
    <polygon {...common} points={points} />
  ) : (
    <polyline {...common} points={points} />
  );
}

/** The same finite vector artwork is compiled into the editable Rive rig. */
export function ActivityProps({
  mood,
  compact,
  config,
  theme,
  surface,
}: {
  mood: AvatarMood;
  compact: boolean;
  config: AvatarConfig;
  theme: AvatarTheme;
  surface?: string;
}) {
  const still = staticPropPose(mood);
  const id = useId().replace(/:/g, "");
  const material = propMaterial(config, theme, surface);
  const textured = !compact && material.surface !== "flat";
  const paint = (colour: string) =>
    ({
      "#f5f0ff": textured ? `url(#${id}-material)` : material.base,
      "#d7c5ff": material.low,
      "#302644": material.ink,
      "#9872ee": material.ink,
      "#f7c76a": textured ? `url(#${id}-material)` : material.base,
      "#bbc7da": material.highlight,
      "#171320": material.shadow,
    })[colour] ?? colour;
  function render(node: PropNode) {
    // Keep the primary prop readable at chat-header size; omit tiny decoration.
    const detail = ["TapLeft", "TapRight", "ClockPin"].includes(node.name);
    return (
      <g
        key={node.name}
        data-rive-prop={node.name}
        transform={propTransform(still[node.name]!)}
      >
        {node.shapes.map((shape, index) => (
          <Fragment key={index}>
            {!compact || !detail ? (
              <Shape
                shape={shape}
                paint={(colour) =>
                  colour === "#9872ee" && node.name.startsWith("Key")
                    ? material.highlight
                    : paint(colour)
                }
              />
            ) : null}
          </Fragment>
        ))}
        {node.children.map(render)}
      </g>
    );
  }
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      data-avatar-props=""
      data-prop-detail={compact ? "compact" : "full"}
      data-prop-palette={config.choices.palette ?? "core-nyx"}
      data-prop-surface={material.surface}
      viewBox={`0 0 ${PROP_ART.viewBox} ${PROP_ART.viewBox}`}
      className="pointer-events-none absolute inset-0 block size-full"
    >
      <defs>
        <pattern
          id={`${id}-material`}
          width={material.surface.startsWith("ascii-") ? 3 : 2.5}
          height={material.surface.startsWith("ascii-") ? 3 : 2.5}
          patternUnits="userSpaceOnUse"
        >
          <rect width="4" height="4" fill={material.base} />
          {material.surface.startsWith("ascii-") ? (
            <circle cx="1.5" cy="1.5" r="0.55" fill={material.highlight} />
          ) : (
            <rect width="1.25" height="1.25" fill={material.highlight} />
          )}
        </pattern>
      </defs>
      {PROP_ART.roots.map(render)}
    </svg>
  );
}

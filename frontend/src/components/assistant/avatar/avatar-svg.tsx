import { planeDataUri } from "../../../../../shared/agent-avatars/png";
import { useId } from "react";
import type { DrawList } from "../../../../../shared/agent-avatars/draw";
export function AvatarSvg({
  scene,
  label,
  className,
  eyeMaterial,
}: {
  scene: DrawList;
  label?: string;
  className?: string;
  eyeMaterial?: DrawList;
}) {
  const id = useId().replace(/:/g, "");
  return (
    <svg
      data-agent-avatar="face-v1"
      viewBox="0 0 512 512"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={!label}
      className={className}
      data-face-surface={scene.surface}
    >
      <defs>
        <clipPath id={`${id}-face`}>
          <path
            data-avatar-eye-mask=""
            d={scene.mask}
            transform={scene.transform}
            fillRule="evenodd"
            clipRule="evenodd"
          />
        </clipPath>
        <clipPath id={`${id}-outside`}>
          <path
            data-avatar-outside-mask=""
            d={scene.outsideMask}
            fillRule="evenodd"
            clipRule="evenodd"
          />
        </clipPath>
        {eyeMaterial ? (
          <clipPath id={`${id}-old-eyes`}>
            <path
              d={scene.mask.slice(scene.headMask.length)}
              transform={scene.transform}
              fillRule="evenodd"
              clipRule="evenodd"
            />
          </clipPath>
        ) : null}
        <clipPath id={`${id}-frame`}>
          {scene.frame === "circle" ? (
            <circle cx="256" cy="256" r="256" />
          ) : (
            <rect
              width="512"
              height="512"
              rx={scene.frame === "tile" ? 96 : 0}
            />
          )}
        </clipPath>
      </defs>
      <g clipPath={`url(#${id}-frame)`}>
        {scene.backgroundPlane ? (
          <image
            href={planeDataUri(scene.backgroundPlane)}
            width="512"
            height="512"
            preserveAspectRatio="none"
          />
        ) : null}
        <g
          transform={`translate(${scene.motion.x} ${scene.motion.y}) translate(256 256) rotate(${scene.motion.rotate}) scale(${scene.motion.scale}) translate(-256 -256)`}
        >
          {scene.backgroundPaths.map((p, i) => (
            <path
              key={`bg-${i}`}
              d={p.d}
              fill={p.fill}
              opacity={p.opacity}
              shapeRendering={p.crisp ? "crispEdges" : undefined}
            />
          ))}
          {eyeMaterial ? (
            <g clipPath={`url(#${id}-old-eyes)`}>
              <g clipPath={`url(#${id}-face)`}>
                {eyeMaterial.foregroundPlane ? (
                  <image
                    href={planeDataUri(eyeMaterial.foregroundPlane)}
                    width="512"
                    height="512"
                    preserveAspectRatio="none"
                  />
                ) : null}
                {eyeMaterial.paths
                  .filter((path) => path.clip === "face")
                  .map((path, index) => (
                    <path
                      key={index}
                      d={path.d}
                      fill={path.fill}
                      opacity={path.opacity}
                      shapeRendering={path.crisp ? "crispEdges" : undefined}
                    />
                  ))}
              </g>
            </g>
          ) : null}
          {scene.foregroundPlane ? (
            <image
              href={planeDataUri(scene.foregroundPlane)}
              width="512"
              height="512"
              preserveAspectRatio="none"
              clipPath={`url(#${id}-face)`}
            />
          ) : null}
          {scene.paths.map((p, i) => (
            <path
              data-avatar-source-mask={
                p.source && p.d === scene.mask ? "" : undefined
              }
              key={i}
              d={p.d}
              fill={p.fill}
              opacity={p.opacity}
              stroke={p.stroke}
              strokeWidth={p.width}
              transform={p.source ? scene.transform : undefined}
              fillRule="nonzero"
              shapeRendering={p.crisp ? "crispEdges" : undefined}
              clipPath={p.clip ? `url(#${id}-${p.clip})` : undefined}
            />
          ))}
        </g>
      </g>
    </svg>
  );
}

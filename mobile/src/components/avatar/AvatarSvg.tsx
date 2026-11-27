import { planeDataUri } from "../../../../shared/agent-avatars/png";
import { useId } from "react";
import Svg, {
  Defs,
  ClipPath,
  Path,
  G,
  Circle,
  Rect,
  Image,
} from "react-native-svg";
import type { DrawList } from "../../../../shared/agent-avatars/draw";
export function AvatarSvg({ scene }: { scene: DrawList }) {
  const id = useId().replace(/:/g, "");
  return (
    <Svg width="100%" height="100%" viewBox="0 0 512 512" accessible={false}>
      <Defs>
        <ClipPath id={`${id}-face`}>
          <Path
            d={scene.mask}
            transform={scene.transform}
            fillRule="evenodd"
            clipRule="evenodd"
          />
        </ClipPath>
        <ClipPath id={`${id}-outside`}>
          <Path d={scene.outsideMask} fillRule="evenodd" clipRule="evenodd" />
        </ClipPath>
        <ClipPath id={`${id}-frame`}>
          {scene.frame === "circle" ? (
            <Circle cx={256} cy={256} r={256} />
          ) : (
            <Rect
              width={512}
              height={512}
              rx={scene.frame === "tile" ? 96 : 0}
            />
          )}
        </ClipPath>
      </Defs>
      <G clipPath={`url(#${id}-frame)`}>
        {scene.backgroundPlane ? (
          <Image
            href={{ uri: planeDataUri(scene.backgroundPlane) }}
            width={512}
            height={512}
            preserveAspectRatio="none"
          />
        ) : null}
        {scene.backgroundPaths.map((p, i) => (
          <Path key={`bg-${i}`} d={p.d} fill={p.fill} opacity={p.opacity} />
        ))}
        {scene.foregroundPlane ? (
          <Image
            href={{ uri: planeDataUri(scene.foregroundPlane) }}
            width={512}
            height={512}
            preserveAspectRatio="none"
            clipPath={`url(#${id}-face)`}
          />
        ) : null}
        {scene.paths.map((p, i) => (
          <Path
            key={i}
            d={p.d}
            fill={p.fill}
            opacity={p.opacity}
            stroke={p.stroke}
            strokeWidth={p.width}
            transform={p.source ? scene.transform : undefined}
            clipPath={p.clip ? `url(#${id}-${p.clip})` : undefined}
          />
        ))}
      </G>
    </Svg>
  );
}

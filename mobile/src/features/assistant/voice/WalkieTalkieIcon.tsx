import Svg, { Line, Rect } from "react-native-svg";

/**
 * A walkie-talkie in the lucide stroke style (lucide has none): antenna, body
 * and speaker grille. Hold to talk uses it; Hands-free uses a phone.
 */
export function WalkieTalkieIcon({
  size = 24,
  color,
  strokeWidth = 2,
}: {
  size?: number;
  color: string;
  strokeWidth?: number;
}) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={color}
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <Line x1={9} y1={2} x2={9} y2={7} />
      <Rect x={6} y={7} width={12} height={15} rx={2.5} />
      <Line x1={9.5} y1={11} x2={14.5} y2={11} />
      <Line x1={9.5} y1={14} x2={14.5} y2={14} />
      <Line x1={9.5} y1={17} x2={14.5} y2={17} />
    </Svg>
  );
}

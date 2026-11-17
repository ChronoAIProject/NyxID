import Svg, { Path } from "react-native-svg";
import {
  ARROW_ICON_GRID,
  ARROW_ICON_STROKE_WIDTH,
  EXPAND_HORIZONTAL_ARROWS,
  RESTORE_HORIZONTAL_ARROWS,
  horizontalArrowPath,
  type HorizontalArrow,
} from "./horizontalArrowGeometry";

/** The rail toggle's icons (see horizontalArrowGeometry). */
function ArrowIcon({
  arrows,
  size = ARROW_ICON_GRID,
  color,
}: {
  arrows: readonly HorizontalArrow[];
  size?: number;
  color: string;
}) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox={`0 0 ${ARROW_ICON_GRID} ${ARROW_ICON_GRID}`}
      fill="none"
      stroke={color}
      strokeWidth={ARROW_ICON_STROKE_WIDTH}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {arrows.map((arrow, index) => (
        <Path key={index} d={horizontalArrowPath(arrow)} />
      ))}
    </Svg>
  );
}

export function ExpandHorizontalIcon(props: { size?: number; color: string }) {
  return <ArrowIcon arrows={EXPAND_HORIZONTAL_ARROWS} {...props} />;
}

export function RestoreHorizontalIcon(props: { size?: number; color: string }) {
  return <ArrowIcon arrows={RESTORE_HORIZONTAL_ARROWS} {...props} />;
}

export function ChartGradients({
  id,
  colors,
  horizontal = false,
}: {
  id: string;
  colors: string[];
  horizontal?: boolean;
}) {
  return (
    <defs>
      {colors.map((color, index) => (
        <linearGradient
          key={`area-${index}`}
          id={`${id}-area-${index}`}
          x1="0"
          y1="0"
          x2="0"
          y2="1"
        >
          <stop
            offset="0%"
            stopColor={color}
            stopOpacity="var(--analytics-area-top)"
          />
          <stop
            offset="55%"
            stopColor={color}
            stopOpacity="var(--analytics-area-middle)"
          />
          <stop offset="100%" stopColor={color} stopOpacity={0} />
        </linearGradient>
      ))}
      {colors.map((color, index) => (
        <linearGradient
          key={`bar-${index}`}
          id={`${id}-bar-${index}`}
          x1="0"
          y1={horizontal ? "0" : "1"}
          x2={horizontal ? "1" : "0"}
          y2="0"
        >
          <stop
            offset="0%"
            stopColor={color}
            stopOpacity="var(--analytics-bar-start)"
          />
          <stop
            offset="100%"
            stopColor={color}
            stopOpacity="var(--analytics-bar-end)"
          />
        </linearGradient>
      ))}
    </defs>
  );
}

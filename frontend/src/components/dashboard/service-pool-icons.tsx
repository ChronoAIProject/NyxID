import {
  createLucideIcon,
  type IconNode,
  type LucideIcon,
  type LucideProps,
} from "lucide-react";
import type { PoolStrategy } from "@/schemas/pools";

function createPoolIcon(
  name: string,
  paths: readonly string[],
  loads: readonly (readonly [number, number])[],
) {
  return createLucideIcon(name, [
    ...paths.map<IconNode[number]>((d, index) => [
      "path",
      { d, key: `route-${index}` },
    ]),
    ...loads.map<IconNode[number]>(([cx, cy]) => [
      "circle",
      { cx: String(cx), cy: String(cy), r: "3", key: `load-${cx}-${cy}` },
    ]),
  ]);
}

const routingLoads = [
  [6, 18],
  [18, 6],
] as const;
const forwardRoute = "M18 9a9 9 0 0 1-9 9";

const strategyIcons: Record<PoolStrategy, LucideIcon> = {
  priority: createPoolIcon(
    "pool-priority",
    ["M6 3v12", forwardRoute],
    routingLoads,
  ),
  round_robin: createPoolIcon(
    "pool-round-robin",
    [
      "M18.152 6.548a8.7 8.7 0 0 1 2.252 3.9",
      "m18.664 8.815 1.74 1.633 .696-2.285",
      "M14.252 21.104a8.7 8.7 0 0 1-4.504 0",
      "m12.032 20.408-2.284.696 1.632 1.741",
      "M3.596 10.448a8.7 8.7 0 0 1 2.252-3.9",
      "m5.305 8.874 .543-2.326-2.323 .47",
    ],
    [
      [12, 4],
      [19.534, 17.05],
      [4.466, 17.05],
    ],
  ),
  weighted: createPoolIcon(
    "pool-weighted",
    ["M12 3v18", "M8 21h8", "M5 7l14-4", "M5 7v6", "M19 3v6"],
    [
      [5, 16],
      [19, 12],
    ],
  ),
};

const Pool = createPoolIcon(
  "service-pool",
  [
    "M14.947 4.562a8 8 0 0 1 3.216 2.337",
    "M19.984 12.504a8 8 0 0 1-1.228 3.781",
    "M13.988 19.749a8 8 0 0 1-3.976 0",
    "M5.244 16.285a8 8 0 0 1-1.228-3.781",
    "M5.837 6.899a8 8 0 0 1 3.216-2.337",
  ],
  [
    [12, 4],
    [19.608, 9.528],
    [16.702, 18.472],
    [7.298, 18.472],
    [4.392, 9.528],
  ],
);

export function ServicePoolIcon(props: LucideProps) {
  return <Pool {...props} aria-hidden="true" focusable="false" />;
}

export function PoolStrategyIcon({
  strategy = "priority",
  ...props
}: LucideProps & { readonly strategy?: PoolStrategy }) {
  const Icon = strategyIcons[strategy];
  return <Icon {...props} aria-hidden="true" focusable="false" />;
}

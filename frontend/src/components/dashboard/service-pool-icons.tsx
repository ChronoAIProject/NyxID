import {
  GitBranch,
  Network,
  RotateCw,
  Scale,
  type LucideIcon,
  type LucideProps,
} from "lucide-react";
import type { PoolStrategy } from "@/schemas/pools";

const strategyIcons: Record<PoolStrategy, LucideIcon> = {
  priority: GitBranch,
  round_robin: RotateCw,
  weighted: Scale,
};

export function ServicePoolIcon(props: LucideProps) {
  return <Network {...props} aria-hidden="true" />;
}

export function PoolStrategyIcon({
  strategy,
  ...props
}: LucideProps & { readonly strategy: PoolStrategy }) {
  const Icon = strategyIcons[strategy];
  return <Icon {...props} aria-hidden="true" />;
}

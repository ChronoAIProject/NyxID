import { useMemo } from "react";
import { useNodes } from "@/hooks/use-nodes";
import { cn } from "@/lib/utils";

type HealthLevel = "healthy" | "warning" | "critical";

export function AmbientStatusLine() {
  const { data: nodes } = useNodes();

  const health: HealthLevel = useMemo(() => {
    if (!nodes) return "healthy";
    const offlineCount = nodes.filter((n) => n.status === "Offline").length;
    const drainingCount = nodes.filter((n) => n.status === "Draining").length;
    if (offlineCount > 0) return "critical";
    if (drainingCount > 0) return "warning";
    return "healthy";
  }, [nodes]);

  return (
    <div
      className={cn(
        "fixed top-0 left-0 right-0 z-[60] h-[2px]",
        health === "critical" && "animate-pulse-subtle",
      )}
      style={{
        background:
          health === "healthy"
            ? "linear-gradient(90deg, transparent 0%, rgba(16,185,129,0.4) 50%, transparent 100%)"
            : health === "warning"
              ? "linear-gradient(90deg, transparent 0%, rgba(245,158,11,0.5) 50%, transparent 100%)"
              : "linear-gradient(90deg, transparent 0%, rgba(239,68,68,0.5) 50%, transparent 100%)",
      }}
      aria-hidden="true"
    />
  );
}

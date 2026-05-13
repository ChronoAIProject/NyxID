import { useMemo } from "react";
import { Link } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { CheckCircle2, ArrowRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { ConstellationViz } from "./constellation-viz";

interface SecurityItem {
  readonly label: string;
  readonly done: boolean;
  readonly weight: number;
}

export function HeroPostureCard() {
  const user = useAuthStore((s) => s.user);
  const { data: apiKeys } = useApiKeys();
  const { data: services } = useKeys();
  const { data: nodes } = useNodes();

  const activeKeys = apiKeys?.filter((k) => k.is_active).length ?? 0;
  const serviceCount = services?.length ?? 0;

  const items: SecurityItem[] = useMemo(
    () => [
      { label: "Email verified", done: !!user?.email_verified, weight: 25 },
      { label: "MFA enabled", done: !!user?.mfa_enabled, weight: 35 },
      { label: "Services connected", done: serviceCount > 0, weight: 20 },
      { label: "Agent keys active", done: activeKeys > 0, weight: 20 },
    ],
    [user, serviceCount, activeKeys],
  );

  const score = useMemo(
    () => items.reduce((sum, item) => sum + (item.done ? item.weight : 0), 0),
    [items],
  );

  const ambientClass =
    score >= 80
      ? "ambient-glow-success"
      : score >= 50
        ? "ambient-glow-warning"
        : "ambient-glow-critical";

  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-2xl border border-border bg-card rim-light-top",
        ambientClass,
      )}
    >
      <div className="flex flex-col gap-8 p-8 md:flex-row md:items-start md:gap-12">
        {/* Left: Score Arc + Checklist */}
        <div className="flex flex-col items-center gap-6 md:items-start md:min-w-[220px]">
          <ScoreArc score={score} />

          <div className="flex flex-col gap-2.5 w-full">
            {items.map((item) => (
              <div key={item.label} className="flex items-center gap-2.5">
                {item.done ? (
                  <CheckCircle2 className="h-4 w-4 shrink-0 text-success" />
                ) : (
                  <div className="h-4 w-4 shrink-0 rounded-full border-2 border-border" />
                )}
                <span
                  className={cn(
                    "text-[13px]",
                    item.done
                      ? "text-muted-foreground"
                      : "text-foreground font-medium",
                  )}
                >
                  {item.label}
                </span>
              </div>
            ))}
          </div>

          {score < 100 && (
            <Link
              to="/settings"
              search={{ tab: undefined }}
              className="flex items-center gap-1.5 text-[12px] font-semibold text-nyx-secondary-400 transition-colors duration-300 hover:text-nyx-300"
            >
              Improve score
              <ArrowRight className="h-3 w-3" />
            </Link>
          )}
        </div>

        {/* Right: Constellation */}
        <div className="flex-1 flex items-center justify-center min-h-[200px] md:min-h-[260px]">
          <ConstellationViz
            services={services ?? []}
            nodes={nodes ?? []}
          />
        </div>
      </div>
    </div>
  );
}

function ScoreArc({ score }: { readonly score: number }) {
  const size = 160;
  const strokeWidth = 8;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const targetOffset = circumference - (score / 100) * circumference;

  const color = score >= 80 ? "#10B981" : score >= 50 ? "#F59E0B" : "#EF4444";

  return (
    <div className="relative" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <defs>
          <linearGradient id="score-gradient" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#A672FB" />
            <stop offset="100%" stopColor="#5A2AF1" />
          </linearGradient>
        </defs>
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="currentColor"
          strokeWidth={strokeWidth}
          className="text-border/50"
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={score >= 80 ? "url(#score-gradient)" : color}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          className="animate-score-draw"
          style={{
            "--circumference": circumference,
            "--target-offset": targetOffset,
            strokeDasharray: circumference,
            strokeDashoffset: circumference,
          } as React.CSSProperties}
        />
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center">
        <span
          className="text-[40px] font-bold leading-none nyx-gradient-text"
          style={{ letterSpacing: "-0.04em" }}
        >
          {score}
        </span>
        <span className="text-[11px] font-medium text-text-tertiary mt-0.5">
          security score
        </span>
      </div>
    </div>
  );
}

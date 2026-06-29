import { Link } from "@tanstack/react-router";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  getStatusMeta,
  type StatusDomain,
} from "@/lib/status-contract";

interface StatusBadgeProps {
  readonly domain: StatusDomain;
  /**
   * Key into `STATUS_REGISTRY[domain]`. If the key is unknown the badge
   * falls back to rendering the raw value with a neutral variant — that's
   * a "lights still on" failure mode rather than a runtime error.
   */
  readonly statusKey: string;
  readonly className?: string;
}

/**
 * Renders the registry-driven status meta as a badge. Hovering shows the
 * one-line meaning + (where applicable) a small remediation link. Wave B
 * item B.4 — replaces the half-dozen bare-badge switches scattered
 * across the app so users can finally learn what each state means
 * without reading the source code.
 *
 * Falls through to `<Badge variant="outline">{statusKey}</Badge>` when
 * the (domain, statusKey) pair is unknown, so a new server-side status
 * never crashes the UI — it just shows the raw key with no tooltip
 * until someone adds it to the registry.
 */
export function StatusBadge({ domain, statusKey, className }: StatusBadgeProps) {
  const meta = getStatusMeta(domain, statusKey);

  if (!meta) {
    return (
      <Badge variant="secondary" className={className}>
        {statusKey}
      </Badge>
    );
  }

  const badge = (
    <Badge variant={meta.variant} className={cn(className, "cursor-help")}>
      {meta.label}
    </Badge>
  );

  return (
    <TooltipProvider delayDuration={150}>
      <Tooltip>
        <TooltipTrigger asChild>
          {/* `<span>` so a click anywhere on the badge doesn't get hijacked
              by a parent button/link; tabIndex makes it keyboard-focusable
              so screen readers can surface the tooltip via focus too. */}
          <span tabIndex={0} className="inline-block">
            {badge}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs space-y-1">
          <p className="text-xs">{meta.tooltip}</p>
          {meta.remediation ? (
            <Link
              to={meta.remediation.href}
              className="inline-block text-[11px] text-primary underline"
            >
              {meta.remediation.label} →
            </Link>
          ) : null}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

import {
  Check,
  Circle,
  ExternalLink,
  KeyRound,
  ShieldCheck,
} from "lucide-react";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { ConnectCardContentBlock } from "@/types/assistant";

const STATE_LABEL: Record<ConnectCardContentBlock["state"], string> = {
  needs_connection: "Not connected",
  waiting_for_provider: "Authorizing",
  waiting_for_user: "Waiting for you",
  connected: "Connected",
  error: "Connection failed",
  timed_out: "Timed out",
};

export function ConnectCard({
  block,
}: {
  readonly block: ConnectCardContentBlock;
}) {
  const connected = block.state === "connected";
  const failed = block.state === "error" || block.state === "timed_out";

  return (
    <section className="overflow-hidden rounded-xl border border-border/70 bg-card">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 px-4 py-3">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
            <ServiceIcon
              slug={`api-${block.catalog_slug}`}
              size="sm"
              className="text-nyx-secondary-400"
            />
          </div>
          <div className="min-w-0">
            <p className="truncate text-[13px] font-semibold text-foreground">
              {block.service_name}
            </p>
            <p className="truncate text-[11px] text-muted-foreground">
              {block.subtitle}
            </p>
          </div>
        </div>
        <Badge
          variant={connected ? "success" : failed ? "destructive" : "secondary"}
        >
          {STATE_LABEL[block.state]}
        </Badge>
      </div>

      <div className="px-4 py-3">
        <ol className="space-y-3">
          {block.steps.map((step, index) => (
            <li key={`${step.title}-${String(index)}`} className="flex gap-3">
              <span
                className={`mt-0.5 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-md border ${
                  step.done
                    ? "border-success/30 bg-success/10 text-success"
                    : "border-hairline bg-overlay text-text-tertiary"
                }`}
              >
                {step.done ? (
                  <Check className="h-3 w-3" />
                ) : (
                  <Circle className="h-2 w-2" />
                )}
              </span>
              <div className="min-w-0">
                <p className="text-[12px] font-medium text-foreground">
                  {step.title}
                </p>
                <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                  {step.body}
                </p>
              </div>
            </li>
          ))}
        </ol>

        {!connected && (
          <div className="mt-4 flex flex-wrap gap-2 pl-[30px]">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button type="button" variant="primary" size="sm">
                  {block.auth_kind === "api_key" ? (
                    <KeyRound />
                  ) : (
                    <ExternalLink />
                  )}
                  Connect {block.service_name}
                </Button>
              </TooltipTrigger>
              <TooltipContent>Wired in the API pass</TooltipContent>
            </Tooltip>
          </div>
        )}

        {block.error_message && (
          <p className="mt-3 rounded-lg border border-destructive/20 bg-destructive/[0.06] px-3 py-2 text-[11px] text-destructive">
            {block.error_message}
          </p>
        )}
      </div>

      <div className="flex items-center gap-2 border-t border-border/50 bg-overlay px-4 py-2 text-[10px] text-text-tertiary">
        <ShieldCheck className="h-3.5 w-3.5" />
        <span>{block.footer}</span>
      </div>
    </section>
  );
}

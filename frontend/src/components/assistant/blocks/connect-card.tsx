import { Link } from "@tanstack/react-router";
import { ExternalLink, KeyRound } from "lucide-react";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { ConnectCardContentBlock } from "@/types/assistant";

const STATE_LABEL: Record<ConnectCardContentBlock["state"], string> = {
  needs_connection: "Not connected",
  waiting_for_provider: "Authorizing",
  waiting_for_user: "Waiting for you",
  connected: "Connected",
  error: "Failed",
  timed_out: "Timed out",
};

/**
 * Compact one-row connection prompt (DESIGN.md banner anatomy: icon tile +
 * text + action, compact density). The icon comes from the same catalog
 * glyph registry the AI Services page uses — `catalog_slug` is the raw
 * NyxID service slug (`api-github`, `llm-openai`) passed straight through.
 */
export function ConnectCard({
  block,
}: {
  readonly block: ConnectCardContentBlock;
}) {
  const connected = block.state === "connected";
  const failed = block.state === "error" || block.state === "timed_out";
  const guidance =
    block.error_message ?? block.steps[0]?.body ?? block.subtitle;

  return (
    <section className="flex items-center gap-3 rounded-xl border border-border/70 bg-card px-4 py-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
        <ServiceIcon slug={block.catalog_slug} size="sm" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <p className="truncate text-[13px] font-semibold text-foreground">
            {block.service_name}
          </p>
          <Badge
            variant={connected ? "success" : failed ? "destructive" : "warning"}
          >
            {STATE_LABEL[block.state]}
          </Badge>
        </div>
        <p className="truncate text-[11px] text-muted-foreground">{guidance}</p>
      </div>
      {!connected && (
        <Button asChild variant="primary" size="sm" className="shrink-0">
          <Link to="/keys">
            {block.auth_kind === "api_key" ? <KeyRound /> : <ExternalLink />}
            Connect
          </Link>
        </Button>
      )}
    </section>
  );
}

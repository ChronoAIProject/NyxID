import { useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  Globe,
  Loader2,
  Server,
  ShieldCheck,
  X,
} from "lucide-react";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  actionServiceLabel,
  clampServiceLabel,
  descriptorForAction,
} from "@/lib/assistant/action-registry";
import type { ActionReport } from "@/schemas/assistant-actions";
import type { ActionCardContentBlock } from "@/types/assistant";

interface ActionCardProps {
  readonly block: ActionCardContentBlock;
  readonly onProgress: (blockId: string, inProgress: boolean) => void;
  readonly onResolve: (report: ActionReport) => Promise<void> | void;
}

function ParameterSummary({
  block,
}: {
  readonly block: ActionCardContentBlock;
}) {
  const params = block.params;
  if (params.variant === "unknown") return null;
  const scopes =
    params.variant === "catalog" ? params.requested_scopes.filter(Boolean) : [];
  let endpointHost = "";
  if (params.variant === "custom" && params.endpoint_url) {
    try {
      endpointHost = new URL(params.endpoint_url).host;
    } catch {
      endpointHost = "";
    }
  }

  return (
    <div className="mt-3 space-y-2 rounded-lg bg-white/[0.03] px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[10px] font-semibold uppercase tracking-[1px] text-text-tertiary">
          Service
        </span>
        <Badge variant="secondary">
          {clampServiceLabel(
            params.variant === "catalog" ? params.service_slug : params.name,
          ) || "Custom"}
        </Badge>
        {endpointHost ? (
          <Badge variant="secondary">
            <Globe className="mr-1 h-3 w-3" />
            {endpointHost}
          </Badge>
        ) : null}
      </div>
      {scopes.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-[1px] text-text-tertiary">
            Scopes
          </span>
          {scopes.map((scope) => (
            <Badge key={scope} variant="secondary" className="font-mono">
              {scope}
            </Badge>
          ))}
        </div>
      ) : null}
      {params.via_node_id || params.target_org_id ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {/* Ids are wire-supplied and only length-capped at 256 chars, so they
              stay inside the card instead of widening the chat column. */}
          {params.via_node_id ? (
            <Badge variant="info" className="max-w-full truncate font-mono">
              <Server className="mr-1 h-3 w-3" />
              Node {params.via_node_id}
            </Badge>
          ) : null}
          {params.target_org_id ? (
            <Badge variant="secondary" className="max-w-full truncate font-mono">
              Org {params.target_org_id}
            </Badge>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

const RECEIPT = {
  completed: {
    title: "Service connected",
    icon: Check,
    container: "border-success/30 bg-success/[0.06]",
    iconClass: "text-success",
  },
  declined: {
    title: "Action declined",
    icon: X,
    container: "border-border bg-overlay",
    iconClass: "text-muted-foreground",
  },
  failed: {
    title: "Connection failed",
    icon: AlertTriangle,
    container: "border-destructive/30 bg-destructive/[0.06]",
    iconClass: "text-destructive",
  },
} as const;

function Receipt({ block }: { readonly block: ActionCardContentBlock }) {
  if (
    block.status !== "completed" &&
    block.status !== "declined" &&
    block.status !== "failed"
  ) {
    return null;
  }
  const receipt = RECEIPT[block.status];
  const Icon = receipt.icon;
  return (
    <section className={`rounded-xl border p-4 ${receipt.container}`}>
      <div className="flex items-center gap-2 text-[12px] font-semibold text-foreground">
        <Icon className={`h-4 w-4 ${receipt.iconClass}`} />
        {receipt.title}
      </div>
      <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
        {block.outcome_note}
      </p>
      <p className="mt-1.5 text-[10px] text-text-tertiary">
        {actionServiceLabel(block.params)}
      </p>
    </section>
  );
}

export function ActionCard({ block, onProgress, onResolve }: ActionCardProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const resolvingRef = useRef(false);
  const descriptor = descriptorForAction(
    block.action,
    block.params,
    block.status !== "unsupported",
  );

  if (
    block.status === "completed" ||
    block.status === "declined" ||
    block.status === "failed"
  ) {
    return <Receipt block={block} />;
  }

  // Trust the descriptor too: a card whose verb has no journey behind it must
  // never render a CTA, whatever status the block carries.
  const unsupported =
    block.status === "unsupported" || descriptor.risk === "unsupported";
  const busy = block.status === "in_progress";
  const params = block.params;

  function setOpen(next: boolean) {
    setDialogOpen(next);
    if (!next && !resolvingRef.current && block.status === "in_progress") {
      onProgress(block.block_id, false);
    }
  }

  function report(
    disposition: "completed" | "declined",
    userServiceId?: string,
  ) {
    resolvingRef.current = true;
    const base = {
      actionRequestId: block.action_request_id,
      originTurnId: block.origin_turn_id,
      disposition,
    } as const;
    void Promise.resolve(
      onResolve(
        disposition === "completed" && userServiceId
          ? {
              ...base,
              resource: { userService: { userServiceId } },
            }
          : base,
      ),
    ).catch(() => {
      // The transport retains failed/rejected reports for retry. This guard
      // only prevents a synchronous validation failure from locking dismissal.
      resolvingRef.current = false;
    });
  }

  return (
    <section className="rounded-xl border border-warning/25 bg-card p-4">
      <div className="flex items-start gap-3">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
          {params.variant === "catalog" ? (
            <ServiceIcon slug={params.service_slug} size="sm" />
          ) : params.variant === "custom" ? (
            <Globe className="h-4 w-4 text-muted-foreground" />
          ) : (
            <AlertTriangle className="h-4 w-4 text-warning" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-[13px] font-semibold text-foreground">
              {descriptor.title(params)}
            </h3>
            <Badge variant={unsupported ? "destructive" : "warning"}>
              {unsupported
                ? "Unsupported"
                : busy
                  ? "In progress"
                  : "Action required"}
            </Badge>
          </div>
          <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">
            {descriptor.body(params)}
          </p>
        </div>
      </div>

      <ParameterSummary block={block} />

      {!unsupported ? (
        <div className="mt-3 flex items-start gap-2 rounded-lg border border-warning/15 bg-warning/[0.04] px-3 py-2.5">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            You choose the account, routing, and credential. The assistant
            receives only brokered access after you finish.
          </p>
        </div>
      ) : null}

      <div className="mt-4 flex flex-wrap items-center gap-2">
        {!unsupported ? (
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={busy}
            onClick={() => {
              onProgress(block.block_id, true);
              setDialogOpen(true);
            }}
          >
            {busy ? <Loader2 className="animate-spin" /> : <ShieldCheck />}
            {busy ? "Connecting" : descriptor.cta(params)}
          </Button>
        ) : null}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => report("declined")}
        >
          <X />
          Decline
        </Button>
        <span className="ml-auto text-[10px] text-text-tertiary">
          Nothing is shared until you finish.
        </span>
      </div>

      {!unsupported ? (
        <AddKeyDialog
          open={dialogOpen}
          onOpenChange={setOpen}
          prefillSlug={
            params.variant === "catalog" ? params.service_slug : undefined
          }
          prefillNodeId={
            params.variant === "unknown"
              ? undefined
              : (params.via_node_id ?? undefined)
          }
          prefillTargetOrgId={
            params.variant === "unknown"
              ? undefined
              : (params.target_org_id ?? undefined)
          }
          prefillCustom={
            params.variant === "custom"
              ? {
                  name: params.name,
                  endpointUrl: params.endpoint_url,
                  authMethod: params.auth_method,
                  authKeyName: params.auth_key_name,
                }
              : undefined
          }
          onSuccess={({ userServiceId }) => report("completed", userServiceId)}
        />
      ) : null}
    </section>
  );
}

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
    <div className="space-y-2.5 border-y border-border bg-muted px-4 py-3">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[10px] font-semibold uppercase tracking-[1px] text-muted-foreground">
          Service
        </span>
        <Badge variant="secondary" className="max-w-full truncate">
          <span className="min-w-0 truncate">
            {clampServiceLabel(
              params.variant === "catalog" ? params.service_slug : params.name,
            ) || "Custom"}
          </span>
        </Badge>
        {endpointHost ? (
          <Badge variant="secondary" className="max-w-full truncate">
            <Globe className="mr-1 h-3 w-3" />
            <span className="min-w-0 truncate">{endpointHost}</span>
          </Badge>
        ) : null}
      </div>
      {scopes.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-[1px] text-muted-foreground">
            Scopes
          </span>
          {scopes.map((scope) => (
            <Badge
              key={scope}
              variant="secondary"
              className="max-w-full truncate font-mono"
            >
              <span className="min-w-0 truncate">{scope}</span>
            </Badge>
          ))}
        </div>
      ) : null}
      {params.via_node_id || params.target_org_id ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {/* Ids are wire-supplied and only length-capped at 256 chars, so they
              stay inside the card instead of widening the chat column. */}
          {params.via_node_id ? (
            <Badge variant="secondary" className="max-w-full truncate font-mono">
              <Server className="mr-1 h-3 w-3" />
              <span className="min-w-0 truncate">
                Node {params.via_node_id}
              </span>
            </Badge>
          ) : null}
          {params.target_org_id ? (
            <Badge variant="secondary" className="max-w-full truncate font-mono">
              <span className="min-w-0 truncate">
                Org {params.target_org_id}
              </span>
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
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm">
      <div className="flex items-start gap-3 px-4 py-3.5">
        <div
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border ${
            unsupported
              ? "border-destructive/30 bg-destructive/10"
              : "border-nyx-secondary-400/30 bg-nyx-secondary-400/10"
          }`}
        >
          {params.variant === "catalog" ? (
            <ServiceIcon slug={params.service_slug} size="sm" />
          ) : params.variant === "custom" ? (
            <Globe className="h-4 w-4 text-muted-foreground" />
          ) : (
            <AlertTriangle className="h-4 w-4 text-destructive" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-[13px] font-semibold text-foreground">
              {descriptor.title(params)}
            </h3>
            <Badge
              variant={unsupported ? "destructive" : "accent"}
            >
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
        <div className="flex items-start gap-2 px-4 py-3">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-nyx-secondary-400" />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            You choose the account, routing, and credential. The assistant
            receives only brokered access after you finish.
          </p>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted px-4 py-3">
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
        <span className="ml-auto text-[10px] text-muted-foreground">
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
          prefillIncludeAllCatalog={params.variant === "catalog"}
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

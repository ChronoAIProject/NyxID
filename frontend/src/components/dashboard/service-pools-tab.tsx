import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useRef, useState } from "react";
import { MoreVertical } from "lucide-react";
import { toast } from "sonner";
import { CopyableField } from "@/components/shared/copyable-field";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useDeleteServicePool,
  useServicePools,
  useUpdateServicePool,
} from "@/hooks/use-pools";
import { useOrgs } from "@/hooks/use-orgs";
import type { ServicePool } from "@/schemas/pools";
import { Choice } from "./pool-controls";
import { PoolEditor } from "./pool-editor";
import { PoolHealthDialog } from "./pool-health-dialog";
import { message, readOnlyPreview, strategyLabels } from "./pool-labels";
import { ServicePoolCards } from "./service-pool-cards";
import { PoolStrategyIcon, ServicePoolIcon } from "./service-pool-icons";

export { PoolEditor } from "./pool-editor";
export { PoolHealthDialog } from "./pool-health-dialog";

interface ServicePoolsTabProps {
  readonly layout?: "cards" | "table";
  readonly initialOrgId?: string;
  readonly initialPoolId?: string;
  readonly createOpen: boolean;
  readonly onCreateOpenChange: (open: boolean) => void;
}
export function ServicePoolsTab({
  layout = "table",
  initialOrgId,
  initialPoolId,
  createOpen,
  onCreateOpenChange,
}: ServicePoolsTabProps) {
  const { t } = useUiTranslation();
  const [owner, setOwner] = useState(initialOrgId ?? "personal");
  const orgId = owner === "personal" ? undefined : owner;
  const { data: orgs } = useOrgs();
  const managedOrgs = (orgs ?? []).filter((o) =>
    ["owner", "admin"].includes(o.your_role),
  );
  const ownerLabel =
    managedOrgs.find((o) => o.id === orgId)?.display_name ||
    managedOrgs.find((o) => o.id === orgId)?.slug ||
    "Personal";
  const pools = useServicePools(orgId);
  const update = useUpdateServicePool();
  const remove = useDeleteServicePool();
  const [editing, setEditing] = useState<ServicePool | null>(null);
  const [health, setHealth] = useState<ServicePool | null>(null);
  const [using, setUsing] = useState<ServicePool | null>(null);
  const [deleting, setDeleting] = useState<ServicePool | null>(null);
  const dialogTrigger = useRef<HTMLButtonElement | null>(null);
  function restoreDialogFocus(event: Event) {
    if (dialogTrigger.current?.isConnected) {
      event.preventDefault();
      dialogTrigger.current.focus();
    }
  }
  async function toggle(pool: ServicePool) {
    try {
      await update.mutateAsync({
        poolId: pool.id,
        expected_revision: pool.config_revision ?? 0,
        is_active: !pool.is_active,
      });
      toast.success(pool.is_active ? "Pool disabled" : "Pool enabled");
    } catch (error) {
      toast.error(translateErrorMessage(message(error)));
    }
  }
  const actions = (pool: ServicePool) => (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="shrink-0"
          aria-label={t("Actions for {{name}}", { name: pool.name })}
          onPointerDown={(event) => {
            dialogTrigger.current = event.currentTarget;
          }}
          onKeyDown={(event) => {
            dialogTrigger.current = event.currentTarget;
          }}
        >
          <MoreVertical className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="data-[state=closed]:hidden"
        onCloseAutoFocus={(event) => {
          // An independent dialog owns focus until it closes.
          if (editing || health || using || deleting) event.preventDefault();
        }}
      >
        <DropdownMenuItem onSelect={() => setUsing(pool)}>{t("Use pool")}</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setEditing(pool)}>{t("Edit")}</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setHealth(pool)}>{t("Connections & health")}</DropdownMenuItem>
        <DropdownMenuItem
          disabled={readOnlyPreview || update.isPending}
          onSelect={() => {
            void toggle(pool);
          }}
        >
          {pool.is_active ? t("Disable") : t("Enable")}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive"
          disabled={readOnlyPreview}
          onSelect={() => setDeleting(pool)}
        >{t("Delete")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <p className="max-w-xl text-12 leading-relaxed text-muted-foreground">{t("Keep one stable name for your apps. Prefer a platform connection, fall back to your own key, or share traffic across compatible services.")}</p>
        {managedOrgs.length > 0 && (
          <div className="min-w-48">
            <Choice
              label={t("Owner")}
              value={owner}
              onChange={(value) => {
                setOwner(value);
                setEditing(null);
                setHealth(null);
                setUsing(null);
              }}
              options={[
                ["personal", "Personal"],
                ...managedOrgs.map(
                  (o) => [o.id, o.display_name || o.slug] as [string, string],
                ),
              ]}
            />
          </div>
        )}
      </div>
      {pools.isError && (
        <ErrorBanner
          message={message(pools.error)}
          onRetry={() => {
            void pools.refetch();
          }}
        />
      )}
      {pools.isLoading && <Skeleton className="h-24" />}
      {pools.data?.length === 0 && (
        <div className="rounded-xl border border-border/50 bg-card px-6 py-12 text-center">
          <div className="mx-auto mb-4 flex size-10 items-center justify-center rounded-xl border border-border/50 bg-overlay">
            <ServicePoolIcon className="size-5 text-muted-foreground" />
          </div>
          <h3 className="text-15 font-semibold">{t("Your connections, one reliable route")}</h3>
          <p className="mx-auto mt-2 max-w-md text-12 leading-relaxed text-muted-foreground">{t("Create a pool from")}{" "}
            {orgId
              ? t("{{ownerLabel}}’s connections", { ownerLabel: ownerLabel })
              : t("your personal connections")}{t(". Put your preferred connection first and add a backup for when it reaches its limit.")}</p>
          <div className="mt-6 flex flex-wrap items-center justify-center gap-3 text-12 text-muted-foreground">
            <span className="rounded-lg border border-border/50 px-3 py-2">{t("1. Choose connections")}</span>
            <span aria-hidden="true">→</span>
            <span className="rounded-lg border border-border/50 px-3 py-2">{t("2. Set their order")}</span>
            <span aria-hidden="true">→</span>
            <span className="rounded-lg border border-border/50 px-3 py-2">{t("3. Use one pool name")}</span>
          </div>
        </div>
      )}
      {(pools.data?.length ?? 0) > 0 && layout === "cards" && (
        <ServicePoolCards
          key={owner}
          pools={pools.data ?? []}
          initialOpenId={orgId === initialOrgId ? initialPoolId : undefined}
          actions={actions}
          onEdit={setEditing}
        />
      )}
      {(pools.data?.length ?? 0) > 0 && layout === "table" && (
        <>
          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table className="table-fixed">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("Name")}</TableHead>
                  <TableHead>{t("Pool slug")}</TableHead>
                  <TableHead>{t("Routing")}</TableHead>
                  <TableHead>{t("Connections")}</TableHead>
                  <TableHead className="w-24">{t("Status")}</TableHead>
                  <TableHead className="w-20">{t("Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pools.data?.map((pool) => (
                  <TableRow key={pool.id}>
                    <TableCell>
                      <button
                        type="button"
                        className="flex max-w-full items-start gap-2 break-words text-left font-medium hover:underline"
                        onClick={(event) => {
                          dialogTrigger.current = event.currentTarget;
                          setEditing(pool);
                        }}
                      >
                        <ServicePoolIcon className="mt-0.5 size-4 shrink-0" />
                        <span>{pool.name}</span>
                      </button>
                      {pool.description && (
                        <p className="mt-1 max-w-64 truncate text-11 text-muted-foreground">
                          {pool.description}
                        </p>
                      )}
                    </TableCell>
                    <TableCell className="break-all font-mono">
                      {pool.slug}
                    </TableCell>
                    <TableCell>
                      <span className="flex items-center gap-1.5">
                        <PoolStrategyIcon
                          strategy={pool.strategy}
                          className="size-3.5 shrink-0"
                        />
                        {strategyLabels[pool.strategy]}
                      </span>
                      <span className="block pl-5 text-11 text-muted-foreground">
                        {pool.member_contract === "ai_chat"
                          ? t("AI chat")
                          : t("Same API")}
                      </span>
                    </TableCell>
                    <TableCell>
                      {pool.members.filter((m) => m.enabled).length} /{" "}
                      {pool.members.length}{t(" enabled")}</TableCell>
                    <TableCell>
                      <Badge variant={pool.is_active ? "success" : "secondary"}>
                        {pool.is_active ? t("Enabled") : t("Disabled")}
                      </Badge>
                    </TableCell>
                    <TableCell>{actions(pool)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          <div className="space-y-3 md:hidden">
            {pools.data?.map((pool) => (
              <div
                key={pool.id}
                className="flex items-center justify-between gap-3 rounded-xl border border-border/50 bg-card p-4"
              >
                <div className="min-w-0 flex-1">
                  <p className="flex items-start gap-2 break-words text-13 font-medium">
                    <ServicePoolIcon className="mt-0.5 size-4 shrink-0" />
                    <span>{pool.name}</span>
                  </p>
                  <p className="break-words text-12 text-muted-foreground">
                    {pool.slug}
                  </p>
                  <p className="flex items-center gap-1.5 text-12 text-muted-foreground">
                    <PoolStrategyIcon
                      strategy={pool.strategy}
                      className="size-3.5 shrink-0"
                    />
                    {strategyLabels[pool.strategy]} ·{" "}
                    {pool.is_active ? t("Enabled") : t("Disabled")}
                  </p>
                </div>
                {actions(pool)}
              </div>
            ))}
          </div>
        </>
      )}
      {createOpen && (
        <PoolEditor
          orgId={orgId}
          ownerLabel={ownerLabel}
          onClose={() => onCreateOpenChange(false)}
        />
      )}
      {editing && (
        <PoolEditor
          key={editing.id}
          pool={editing}
          ownerLabel={ownerLabel}
          orgId={orgId}
          onClose={() => setEditing(null)}
          onCloseAutoFocus={restoreDialogFocus}
        />
      )}
      {health && (
        <PoolHealthDialog
          pool={health}
          onClose={() => setHealth(null)}
          onCloseAutoFocus={restoreDialogFocus}
        />
      )}
      {using && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setUsing(null);
          }}
        >
          <DialogContent onCloseAutoFocus={restoreDialogFocus}>
            <DialogHeader>
              <DialogTitle className="break-words">{t("Use ")}{using.name}
              </DialogTitle>
              <DialogDescription className="text-12">{t("Use this pool name wherever you would call a connection. Each request still needs permission to access the selected connection.")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              {orgId && (
                <p className="text-12 text-muted-foreground">{t("A personal connection or pool with the same slug takes precedence. Give this organization pool a unique slug to select it reliably.")}</p>
              )}
              <CopyableField label={t("Pool slug")} value={using.slug} />
              <CopyableField
                label={t("Proxy path")}
                value={`/api/v1/proxy/s/${using.slug}${using.member_contract === "ai_chat" ? "/chat/completions" : ""}`}
              />
              <p className="text-12 text-muted-foreground">
                {using.member_contract === "ai_chat"
                  ? t("Send an OpenAI chat-completions request. The pool chooses the model configured for each connection.")
                  : t("Append your API operation path and send the same method and body your connections expect.")}
              </p>
              {using.member_contract === "ai_chat" && (
                <>
                  <CopyableField
                    label={t("Gateway model")}
                    value={`pool:${using.slug}`}
                  />
                  <p className="text-12 text-muted-foreground">{t("Use this model with the NyxID LLM gateway’s chat-completions endpoint.")}</p>
                </>
              )}
            </div>
            <DialogFooter>
              <Button onClick={() => setUsing(null)}>{t("Close")}</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
      {deleting && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setDeleting(null);
          }}
        >
          <DialogContent onCloseAutoFocus={restoreDialogFocus}>
            <DialogHeader>
              <DialogTitle className="break-words">{t("Delete ")}{deleting.name}?
              </DialogTitle>
              <DialogDescription>{t("The pool route will stop working. Member connections stay available.")}</DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setDeleting(null)}>{t("Cancel")}</Button>
              <Button
                variant="destructive"
                isLoading={remove.isPending}
                onClick={() => {
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => {
                      setDeleting(null);
                      toast.success(translateUi("Pool deleted"));
                    })
                    .catch((error) => toast.error(translateErrorMessage(message(error))));
                }}
              >{t("Delete")}</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}

import { useRef, useState } from "react";
import { Layers, MoreVertical } from "lucide-react";
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
import { message, strategyLabels } from "./pool-labels";

export { PoolEditor } from "./pool-editor";
export { PoolHealthDialog } from "./pool-health-dialog";

interface ServicePoolsTabProps {
  readonly createOpen: boolean;
  readonly onCreateOpenChange: (open: boolean) => void;
}
export function ServicePoolsTab({
  createOpen,
  onCreateOpenChange,
}: ServicePoolsTabProps) {
  const [owner, setOwner] = useState("personal");
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
      toast.error(message(error));
    }
  }
  const actions = (pool: ServicePool) => (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="shrink-0"
          aria-label={`Actions for ${pool.name}`}
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
        <DropdownMenuItem onSelect={() => setUsing(pool)}>
          Use pool
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setEditing(pool)}>
          Edit
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setHealth(pool)}>
          Connections & health
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => {
            void toggle(pool);
          }}
        >
          {pool.is_active ? "Disable" : "Enable"}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive"
          onSelect={() => setDeleting(pool)}
        >
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <p className="max-w-xl text-[12px] leading-relaxed text-muted-foreground">
          Keep one stable name for your apps. Prefer a platform connection, fall
          back to your own key, or share traffic across compatible services.
        </p>
        {managedOrgs.length > 0 && (
          <div className="min-w-48">
            <Choice
              label="Owner"
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
          <div className="mx-auto mb-4 flex size-10 items-center justify-center rounded-xl border border-border/50 bg-white/[0.03]">
            <Layers className="size-5 text-muted-foreground" />
          </div>
          <h3 className="text-[15px] font-semibold">
            Your connections, one reliable route
          </h3>
          <p className="mx-auto mt-2 max-w-md text-[12px] leading-relaxed text-muted-foreground">
            Create a pool from{" "}
            {orgId
              ? `${ownerLabel}’s connections`
              : "your personal connections"}
            . Put your preferred connection first and add a backup for when it
            reaches its limit.
          </p>
          <div className="mt-6 flex flex-wrap items-center justify-center gap-3 text-[12px] text-muted-foreground">
            <span className="rounded-lg border border-border/50 px-3 py-2">
              1. Choose connections
            </span>
            <span aria-hidden="true">→</span>
            <span className="rounded-lg border border-border/50 px-3 py-2">
              2. Set their order
            </span>
            <span aria-hidden="true">→</span>
            <span className="rounded-lg border border-border/50 px-3 py-2">
              3. Use one pool name
            </span>
          </div>
        </div>
      )}
      {(pools.data?.length ?? 0) > 0 && (
        <>
          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table className="table-fixed">
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Pool slug</TableHead>
                  <TableHead>Routing</TableHead>
                  <TableHead>Connections</TableHead>
                  <TableHead className="w-24">Status</TableHead>
                  <TableHead className="w-20">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pools.data?.map((pool) => (
                  <TableRow key={pool.id}>
                    <TableCell>
                      <button
                        type="button"
                        className="max-w-full break-words text-left font-medium hover:underline"
                        onClick={(event) => {
                          dialogTrigger.current = event.currentTarget;
                          setEditing(pool);
                        }}
                      >
                        {pool.name}
                      </button>
                      {pool.description && (
                        <p className="mt-1 max-w-64 truncate text-[11px] text-muted-foreground">
                          {pool.description}
                        </p>
                      )}
                    </TableCell>
                    <TableCell className="break-all font-mono">
                      {pool.slug}
                    </TableCell>
                    <TableCell>
                      {strategyLabels[pool.strategy]}
                      <br />
                      <span className="text-[11px] text-muted-foreground">
                        {pool.member_contract === "ai_chat"
                          ? "AI chat"
                          : "Same API"}
                      </span>
                    </TableCell>
                    <TableCell>
                      {pool.members.filter((m) => m.enabled).length} /{" "}
                      {pool.members.length} enabled
                    </TableCell>
                    <TableCell>
                      <Badge variant={pool.is_active ? "success" : "secondary"}>
                        {pool.is_active ? "Enabled" : "Disabled"}
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
                  <p className="break-words text-[13px] font-medium">
                    {pool.name}
                  </p>
                  <p className="break-words text-[12px] text-muted-foreground">
                    {pool.slug} · {strategyLabels[pool.strategy]} ·{" "}
                    {pool.is_active ? "Enabled" : "Disabled"}
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
              <DialogTitle className="break-words">
                Use {using.name}
              </DialogTitle>
              <DialogDescription className="text-[12px]">
                Use this pool name wherever you would call a connection. Each
                request still needs permission to access the selected
                connection.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              {orgId && (
                <p className="text-[12px] text-muted-foreground">
                  A personal connection or pool with the same slug takes
                  precedence. Give this organization pool a unique slug to
                  select it reliably.
                </p>
              )}
              <CopyableField label="Pool slug" value={using.slug} />
              <CopyableField
                label="Proxy path"
                value={`/api/v1/proxy/s/${using.slug}${using.member_contract === "ai_chat" ? "/chat/completions" : ""}`}
              />
              <p className="text-[12px] text-muted-foreground">
                {using.member_contract === "ai_chat"
                  ? "Send an OpenAI chat-completions request. The pool chooses the model configured for each connection."
                  : "Append your API operation path and send the same method and body your connections expect."}
              </p>
              {using.member_contract === "ai_chat" && (
                <>
                  <CopyableField
                    label="Gateway model"
                    value={`pool:${using.slug}`}
                  />
                  <p className="text-[12px] text-muted-foreground">
                    Use this model with the NyxID LLM gateway’s chat-completions
                    endpoint.
                  </p>
                </>
              )}
            </div>
            <DialogFooter>
              <Button onClick={() => setUsing(null)}>Close</Button>
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
              <DialogTitle className="break-words">
                Delete {deleting.name}?
              </DialogTitle>
              <DialogDescription>
                The pool route will stop working. Member connections stay
                available.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setDeleting(null)}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                isLoading={remove.isPending}
                onClick={() => {
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => {
                      setDeleting(null);
                      toast.success("Pool deleted");
                    })
                    .catch((error) => toast.error(message(error)));
                }}
              >
                Delete
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}

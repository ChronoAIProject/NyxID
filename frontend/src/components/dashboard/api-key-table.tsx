import { useState, useEffect, useCallback } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  useAllAdminedApiKeys,
  useDeleteApiKey,
  useRotateApiKey,
} from "@/hooks/use-api-keys";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import type { ApiKey } from "@/types/api";
import { formatDate, formatRelativeTime, copyToClipboard } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { MoreHorizontal, RefreshCw, Trash2, Copy, Check, KeyRound } from "lucide-react";
import { toast } from "sonner";

function parseScopesString(scopes: string): readonly string[] {
  if (!scopes || scopes.trim().length === 0) return [];
  return scopes.trim().split(/\s+/);
}

function servicesSummary(key: ApiKey): string {
  if (key.allow_all_services) return "All services";
  const count = key.allowed_service_ids?.length ?? 0;
  if (count === 0) return "—";
  return `${String(count)} service${count === 1 ? "" : "s"}`;
}

function scopeBadgeVariant(scope: string): "info" | "warning" {
  return scope === "write" || scope === "delete" ? "warning" : "info";
}

export function ApiKeyTable() {
  const navigate = useNavigate();
  const { data: apiKeys, isLoading } = useAllAdminedApiKeys();
  const deleteMutation = useDeleteApiKey();
  const rotateMutation = useRotateApiKey();
  const [deleteTarget, setDeleteTarget] = useState<ApiKey | null>(null);
  const [newKeyValue, setNewKeyValue] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);

  async function handleDelete() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      toast.success("API key revoked successfully");
      setDeleteTarget(null);
    } catch {
      toast.error("Failed to revoke API key");
    }
  }

  async function handleRotate(key: ApiKey) {
    try {
      const result = await rotateMutation.mutateAsync(key.id);
      setNewKeyValue(result.full_key);
      toast.success("API key rotated successfully");
    } catch {
      toast.error("Failed to rotate API key");
    }
  }

  const handleCopyKey = useCallback(async () => {
    if (!newKeyValue) return;
    try {
      await copyToClipboard(newKeyValue);
      setCopied(true);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }, [newKeyValue]);

  if (isLoading) {
    return (
      <div className="space-y-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={`skel-${String(i)}`} className="h-12 w-full" />
        ))}
      </div>
    );
  }

  if (!apiKeys || apiKeys.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
        <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
          <KeyRound className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="space-y-1">
          <p className="text-[12px] font-medium">No API keys yet</p>
          <p className="text-xs text-muted-foreground">
            Create one to get started.
          </p>
        </div>
      </div>
    );
  }

  return (
    <>
      {/* Mobile card view */}
      <div className="flex flex-col gap-3 md:hidden">
        {apiKeys.map((key) => {
          const scopesList = parseScopesString(key.scopes);
          const source = key.credential_source;
          const isOrg = source?.type === "org";
          return (
            <div
              key={key.id}
              role="button"
              tabIndex={0}
              onClick={() => void navigate({ to: "/keys/api-key/$keyId", params: { keyId: key.id } })}
              onKeyDown={(e) => { if (e.key === "Enter") void navigate({ to: "/keys/api-key/$keyId", params: { keyId: key.id } }); }}
              className="relative rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-white/[0.03] cursor-pointer"
            >
              <div className="absolute right-3 top-3" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon" className="h-7 w-7">
                      <MoreHorizontal className="h-3.5 w-3.5" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => void handleRotate(key)} disabled={!key.is_active}>
                      <RefreshCw className="mr-2 h-4 w-4" /> Rotate
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => setDeleteTarget(key)} className="text-destructive focus:text-destructive" disabled={!key.is_active}>
                      <Trash2 className="mr-2 h-4 w-4 text-destructive" /> Revoke
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
              <p className="pr-10 text-[13px] font-semibold text-foreground truncate">{key.name}</p>
              <code className="text-[11px] font-mono text-muted-foreground">{key.key_prefix}••••••••</code>
              <div className="mt-2 flex flex-wrap gap-1">
                {key.platform && <Badge variant="secondary">{key.platform}</Badge>}
                {scopesList.map((scope) => (
                  <Badge key={scope} variant={scopeBadgeVariant(scope)}>
                    {scope.charAt(0).toUpperCase() + scope.slice(1)}
                  </Badge>
                ))}
              </div>
              <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                <span>{isOrg ? source.org_name : "Personal"}</span>
                <span>{servicesSummary(key)}</span>
                <span>{key.last_used_at ? `Used ${formatRelativeTime(key.last_used_at)}` : "Never used"}</span>
              </div>
            </div>
          );
        })}
      </div>

      {/* Desktop table view */}
      <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="border-border/50 hover:bg-transparent">
              <TableHead className="w-[22%]">Name</TableHead>
              <TableHead className="w-[14%]">Key</TableHead>
              <TableHead className="w-[12%]">Platform</TableHead>
              <TableHead className="w-[14%]">Scopes</TableHead>
              <TableHead className="w-[10%]">Services</TableHead>
              <TableHead className="w-[10%]">Bindings</TableHead>
              <TableHead className="w-[14%]">Last Used</TableHead>
              <TableHead className="w-10">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {apiKeys.map((key) => {
              const scopesList = parseScopesString(key.scopes);
              const source = key.credential_source;
              const isOrg = source?.type === "org";
              const ownerLabel = isOrg ? source.org_name : "Personal";
              return (
                <TableRow
                  key={key.id}
                  className="border-border/30 cursor-pointer hover:bg-white/[0.03]"
                  onClick={() => void navigate({ to: "/keys/api-key/$keyId", params: { keyId: key.id } })}
                >
                  <TableCell>
                    <p className="truncate font-medium text-foreground">
                      {key.name}
                    </p>
                    <p className="truncate text-[11px] text-text-tertiary mt-0.5">
                      {isOrg ? (
                        <span className="inline-flex items-center gap-1">
                          <OrgAvatar
                            avatarUrl={source.avatar_url ?? null}
                            displayName={source.org_name}
                            className="h-3.5 w-3.5 text-[0.5rem]"
                          />
                          {ownerLabel}
                        </span>
                      ) : (
                        ownerLabel
                      )}
                    </p>
                  </TableCell>

                  <TableCell>
                    <code className="font-mono text-[11px] text-muted-foreground">
                      {key.key_prefix}••••••••
                    </code>
                  </TableCell>

                  <TableCell className="whitespace-nowrap">
                    {key.platform ? (
                      <Badge variant="secondary">{key.platform}</Badge>
                    ) : (
                      <span className="text-text-tertiary">—</span>
                    )}
                  </TableCell>

                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {scopesList.length > 0 ? (
                        scopesList.map((scope) => (
                          <Badge key={scope} variant={scopeBadgeVariant(scope)}>
                            {scope.charAt(0).toUpperCase() + scope.slice(1)}
                          </Badge>
                        ))
                      ) : (
                        <span className="text-text-tertiary">—</span>
                      )}
                    </div>
                  </TableCell>

                  <TableCell className="text-muted-foreground">
                    {servicesSummary(key)}
                  </TableCell>

                  <TableCell className="text-muted-foreground">
                    {key.bindings_count > 0
                      ? `${String(key.bindings_count)} binding${key.bindings_count === 1 ? "" : "s"}`
                      : <span className="text-text-tertiary">—</span>}
                  </TableCell>

                  <TableCell>
                    <p className="text-muted-foreground">
                      {key.last_used_at
                        ? formatRelativeTime(key.last_used_at)
                        : <span className="text-text-tertiary">—</span>}
                    </p>
                    <p className="text-[10px] text-text-tertiary mt-0.5">
                      Created {formatDate(key.created_at)}
                    </p>
                  </TableCell>

                  <TableCell onClick={(e) => e.stopPropagation()}>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7">
                          <MoreHorizontal className="h-3.5 w-3.5" aria-hidden="true" />
                          <span className="sr-only">Actions for {key.name}</span>
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          onClick={() => void handleRotate(key)}
                          disabled={!key.is_active}
                        >
                          <RefreshCw className="mr-2 h-4 w-4" aria-hidden="true" />
                          Rotate
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          onClick={() => setDeleteTarget(key)}
                          className="text-destructive focus:text-destructive"
                          disabled={!key.is_active}
                        >
                          <Trash2 className="mr-2 h-4 w-4 text-destructive" aria-hidden="true" />
                          Revoke
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke API Key</DialogTitle>
            <DialogDescription>
              Are you sure you want to revoke &quot;{deleteTarget?.name}&quot;?
              This action cannot be undone and any applications using this key
              will stop working.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Revoke key
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={newKeyValue !== null}
        onOpenChange={(open) => {
          if (!open) {
            setNewKeyValue(null);
            setCopied(false);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New API Key</DialogTitle>
            <DialogDescription>
              Copy your new API key now. You will not be able to see it again.
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2">
            <code className="flex-1 rounded-lg bg-muted p-3 font-mono text-[12px] break-all select-all">
              {newKeyValue}
            </code>
            <Button
              variant="outline"
              size="icon"
              onClick={() => void handleCopyKey()}
              aria-label="Copy API key to clipboard"
            >
              {copied ? (
                <Check className="h-4 w-4 text-success" aria-hidden="true" />
              ) : (
                <Copy className="h-4 w-4" aria-hidden="true" />
              )}
            </Button>
          </div>
          <DialogFooter>
            <Button
              variant="primary"
              onClick={() => {
                setNewKeyValue(null);
                setCopied(false);
              }}
            >
              Done
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

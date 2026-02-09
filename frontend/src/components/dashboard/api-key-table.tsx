import { useState, useEffect, useCallback } from "react"
import { useApiKeys, useDeleteApiKey, useRotateApiKey } from "@/hooks/use-api-keys"
import type { ApiKey } from "@/types/api"
import { maskApiKey, formatDate, formatRelativeTime, copyToClipboard } from "@/lib/utils"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { MoreHorizontal, RefreshCw, Trash2, Copy, Check } from "lucide-react"
import { toast } from "sonner"

function parseScopesString(scopes: string): readonly string[] {
  if (!scopes || scopes.trim().length === 0) return []
  return scopes.trim().split(/\s+/)
}

export function ApiKeyTable() {
  const { data: apiKeys, isLoading } = useApiKeys()
  const deleteMutation = useDeleteApiKey()
  const rotateMutation = useRotateApiKey()
  const [deleteTarget, setDeleteTarget] = useState<ApiKey | null>(null)
  const [newKeyValue, setNewKeyValue] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])

  async function handleDelete() {
    if (!deleteTarget) return
    try {
      await deleteMutation.mutateAsync(deleteTarget.id)
      toast.success("API key revoked successfully")
      setDeleteTarget(null)
    } catch {
      toast.error("Failed to revoke API key")
    }
  }

  async function handleRotate(key: ApiKey) {
    try {
      const result = await rotateMutation.mutateAsync(key.id)
      setNewKeyValue(result.key)
      toast.success("API key rotated successfully")
    } catch {
      toast.error("Failed to rotate API key")
    }
  }

  const handleCopyKey = useCallback(async () => {
    if (!newKeyValue) return
    try {
      await copyToClipboard(newKeyValue)
      setCopied(true)
    } catch {
      toast.error("Failed to copy to clipboard")
    }
  }, [newKeyValue])

  if (isLoading) {
    return (
      <div className="space-y-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={`skel-${String(i)}`} className="h-12 w-full" />
        ))}
      </div>
    )
  }

  if (!apiKeys || apiKeys.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <p className="text-sm text-muted-foreground">
          No API keys yet. Create one to get started.
        </p>
      </div>
    )
  }

  return (
    <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Key</TableHead>
            <TableHead>Scopes</TableHead>
            <TableHead>Created</TableHead>
            <TableHead>Last Used</TableHead>
            <TableHead className="w-12"><span className="sr-only">Actions</span></TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {apiKeys.map((key) => {
            const scopesList = parseScopesString(key.scopes)
            return (
              <TableRow key={key.id}>
                <TableCell className="font-medium">{key.name}</TableCell>
                <TableCell>
                  <code className="rounded bg-muted px-2 py-1 font-mono text-xs">
                    {maskApiKey(key.key_prefix)}
                  </code>
                </TableCell>
                <TableCell>
                  <div className="flex flex-wrap gap-1">
                    {scopesList.map((scope) => (
                      <Badge key={scope} variant="secondary" className="text-xs">
                        {scope}
                      </Badge>
                    ))}
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatDate(key.created_at)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {key.last_used_at
                    ? formatRelativeTime(key.last_used_at)
                    : "Never"}
                </TableCell>
                <TableCell>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon" className="h-8 w-8">
                        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
                        <span className="sr-only">Actions for {key.name}</span>
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onClick={() => void handleRotate(key)}
                        disabled={key.revoked}
                      >
                        <RefreshCw className="mr-2 h-4 w-4" aria-hidden="true" />
                        Rotate
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onClick={() => setDeleteTarget(key)}
                        className="text-destructive focus:text-destructive"
                        disabled={key.revoked}
                      >
                        <Trash2 className="mr-2 h-4 w-4" aria-hidden="true" />
                        Revoke
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null)
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
            <Button
              variant="outline"
              onClick={() => setDeleteTarget(null)}
            >
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
            setNewKeyValue(null)
            setCopied(false)
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
            <code className="flex-1 rounded-md bg-muted p-3 font-mono text-sm break-all select-all">
              {newKeyValue}
            </code>
            <Button
              variant="outline"
              size="icon"
              onClick={() => void handleCopyKey()}
              aria-label="Copy API key to clipboard"
            >
              {copied ? (
                <Check className="h-4 w-4 text-emerald-400" aria-hidden="true" />
              ) : (
                <Copy className="h-4 w-4" aria-hidden="true" />
              )}
            </Button>
          </div>
          <DialogFooter>
            <Button
              onClick={() => {
                setNewKeyValue(null)
                setCopied(false)
              }}
            >
              Done
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

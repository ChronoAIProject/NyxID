import { useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { useNodes } from "@/hooks/use-nodes";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { HardDrive, Pencil } from "lucide-react";
import { toast } from "sonner";

export function NodeScopeCard({
  keyId,
  allowAllNodes,
  allowedNodeIds,
  allowedNodes,
}: {
  readonly keyId: string;
  readonly allowAllNodes: boolean;
  readonly allowedNodeIds: readonly string[];
  readonly allowedNodes: readonly {
    readonly id: string;
    readonly name: string;
    readonly status: string;
  }[];
}) {
  const [editing, setEditing] = useState(false);
  const [allowAll, setAllowAll] = useState(allowAllNodes);
  const [selectedIds, setSelectedIds] =
    useState<readonly string[]>(allowedNodeIds);
  const { data: allNodes } = useNodes();
  const updateApiKey = useUpdateApiKey();

  function handleSave() {
    updateApiKey.mutate(
      {
        keyId,
        allow_all_nodes: allowAll,
        allowed_node_ids: allowAll ? [] : [...selectedIds],
      },
      {
        onSuccess: () => {
          toast.success("Node scope updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update scope";
          toast.error(message);
        },
      },
    );
  }

  function handleCancel() {
    setAllowAll(allowAllNodes);
    setSelectedIds(allowedNodeIds);
    setEditing(false);
  }

  function toggleNode(nodeId: string) {
    setSelectedIds((prev) =>
      prev.includes(nodeId)
        ? prev.filter((id) => id !== nodeId)
        : [...prev, nodeId],
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <HardDrive className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Node Scope</CardTitle>
          </div>
          {!editing && (
            <Button
              size="icon"
              variant="ghost"
              onClick={() => setEditing(true)}
            >
              <Pencil className="h-4 w-4" />
            </Button>
          )}
        </div>
        <CardDescription>
          Which nodes this key can route through
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {editing ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Checkbox
                id="allow-all-nodes"
                checked={allowAll}
                onCheckedChange={(checked) => setAllowAll(checked === true)}
              />
              <Label htmlFor="allow-all-nodes" className="text-[12px]">
                Allow all nodes
              </Label>
            </div>

            {!allowAll && (
              <div className="space-y-2 rounded-lg border border-border p-3">
                <p className="text-xs text-muted-foreground">
                  Select allowed nodes:
                </p>
                {allNodes && allNodes.length > 0 ? (
                  allNodes.map((n) => (
                    <div key={n.id} className="flex items-center gap-2">
                      <Checkbox
                        id={`node-${n.id}`}
                        checked={selectedIds.includes(n.id)}
                        onCheckedChange={() => toggleNode(n.id)}
                      />
                      <Label htmlFor={`node-${n.id}`} className="text-xs">
                        {n.name}
                        <Badge
                          variant={
                            n.status === "online" ? "default" : "secondary"
                          }
                          className="ml-1 text-[10px]"
                        >
                          {n.status}
                        </Badge>
                      </Label>
                    </div>
                  ))
                ) : (
                  <p className="text-xs text-muted-foreground">
                    No nodes registered yet.
                  </p>
                )}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <Button
                variant="primary"
                onClick={handleSave}
                disabled={updateApiKey.isPending}
              >
                Save
              </Button>
              <Button variant="outline" onClick={handleCancel}>
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {allowAllNodes ? (
              <Badge variant="secondary">All nodes</Badge>
            ) : allowedNodes.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {allowedNodes.map((n) => (
                  <Badge key={n.id} variant="secondary" className="text-xs">
                    {n.name}
                  </Badge>
                ))}
              </div>
            ) : (
              <Badge variant="secondary">Direct only (no nodes)</Badge>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

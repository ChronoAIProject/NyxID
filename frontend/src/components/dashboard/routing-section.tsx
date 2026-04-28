import { useState } from "react";
import { Router } from "lucide-react";
import { toast } from "sonner";

import { useNodes } from "@/hooks/use-nodes";
import { useUpdateUserService } from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

interface RoutingSectionProps {
  /** The viewer's `UserService.node_id` for this binding (or `null` for direct routing). */
  readonly nodeId: string | null;
  /** The `UserService._id` to mutate via `PUT /api/v1/keys/{id}`. */
  readonly serviceId: string;
  readonly readOnly?: boolean;
  /**
   * Card title. Defaults to `"Routing"` (the original key-detail
   * wording). The admin services surface uses `"Your Routing"` to
   * signal viewer-scoped semantics — same data, different label.
   */
  readonly title?: string;
  /** Card description. Defaults match the original key-detail copy. */
  readonly description?: string;
}

/**
 * Editable routing section for a single `UserService` (issue #416).
 *
 * Reused by `/keys/$id` (the original surface) and `/services/$id`
 * (the admin surface). The mutation always targets the underlying
 * `UserService` regardless of which page hosts the component, so
 * routing changes stay consistent across surfaces.
 */
export function RoutingSection({
  nodeId,
  serviceId,
  readOnly = false,
  title = "Routing",
  description = "How requests reach the endpoint",
}: RoutingSectionProps) {
  const [picking, setPicking] = useState(false);
  const { data: nodes } = useNodes();
  const updateService = useUpdateUserService();

  function handleSelectNode(selectedNodeId: string) {
    const id = selectedNodeId === "direct" ? "" : selectedNodeId;
    updateService.mutate(
      { serviceId, node_id: id },
      {
        onSuccess: () => {
          toast.success(id ? "Route updated" : "Switched to direct routing");
          setPicking(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update routing";
          toast.error(message);
        },
      },
    );
  }

  const allNodes = nodes ?? [];
  const currentNodeName = nodeId
    ? (nodes?.find((n) => n.id === nodeId)?.name ?? nodeId)
    : null;

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Router className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">{title}</CardTitle>
        </div>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2">
          <Badge variant={nodeId ? "default" : "outline"}>
            {nodeId ? `Via node: ${currentNodeName}` : "Direct"}
          </Badge>
        </div>

        {!readOnly && picking ? (
          <div className="space-y-2">
            <Label className="text-xs">Select routing</Label>
            <Select
              onValueChange={handleSelectNode}
              defaultValue={nodeId ?? "direct"}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select routing" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="direct">Direct (no node)</SelectItem>
                {allNodes.map((n) => (
                  <SelectItem
                    key={n.id}
                    value={n.id}
                    disabled={n.status !== "online"}
                  >
                    {n.name} ({n.status})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {allNodes.length === 0 && (
              <p className="text-xs text-muted-foreground">
                No nodes registered. Register a node first.
              </p>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() => setPicking(false)}
            >
              Cancel
            </Button>
          </div>
        ) : !readOnly ? (
          <Button size="sm" variant="outline" onClick={() => setPicking(true)}>
            {nodeId ? "Change Route" : "Route via Node"}
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}

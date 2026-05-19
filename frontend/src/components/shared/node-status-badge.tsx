import { Badge } from "@/components/ui/badge";

export function NodeStatusBadge({
  status,
  isConnected,
}: {
  readonly status: string;
  readonly isConnected: boolean;
}) {
  if (isConnected) {
    return <Badge variant="success">Online</Badge>;
  }
  if (status === "draining") {
    return <Badge variant="warning">Draining</Badge>;
  }
  return <Badge variant="secondary">Offline</Badge>;
}

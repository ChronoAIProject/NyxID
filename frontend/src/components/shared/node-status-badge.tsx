import { Badge } from "@/components/ui/badge";
import { Wifi, WifiOff } from "lucide-react";

export function NodeStatusBadge({
  status,
  isConnected,
}: {
  readonly status: string;
  readonly isConnected: boolean;
}) {
  if (isConnected) {
    return (
      <Badge variant="success" className="gap-1">
        <Wifi className="h-3 w-3" />
        Online
      </Badge>
    );
  }
  if (status === "draining") {
    return (
      <Badge variant="warning" className="gap-1">
        <WifiOff className="h-3 w-3" />
        Draining
      </Badge>
    );
  }
  return (
    <Badge variant="secondary" className="gap-1">
      <WifiOff className="h-3 w-3" />
      Offline
    </Badge>
  );
}

import type { DownstreamService } from "@/types/api";
import { getAuthTypeLabel, isOidcService } from "@/lib/constants";
import { formatDate } from "@/lib/utils";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Server, Trash2 } from "lucide-react";

interface ServiceCardProps {
  readonly service: DownstreamService;
  readonly onDelete: (id: string) => void;
  readonly isDeleting: boolean;
  readonly onClick: () => void;
}

export function ServiceCard({
  service,
  onDelete,
  isDeleting,
  onClick,
}: ServiceCardProps) {
  return (
    <Card
      className="cursor-pointer transition-colors hover:border-border/80"
      onClick={onClick}
    >
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-3">
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
            <Server className="h-5 w-5 text-primary" />
          </div>
          <div>
            <CardTitle className="text-base">{service.name}</CardTitle>
            <CardDescription className="text-xs">
              {service.base_url}
            </CardDescription>
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 text-muted-foreground hover:text-destructive"
          onClick={(e) => {
            e.stopPropagation();
            onDelete(service.id);
          }}
          disabled={isDeleting}
        >
          <Trash2 className="h-4 w-4" />
          <span className="sr-only">Delete service</span>
        </Button>
      </CardHeader>
      <CardContent>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Badge variant="secondary">{getAuthTypeLabel(service)}</Badge>
            {isOidcService(service) && (
              <Badge variant="outline">OIDC</Badge>
            )}
          </div>
          <span className="text-xs text-muted-foreground">
            Created {formatDate(service.created_at)}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

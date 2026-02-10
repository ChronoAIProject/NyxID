import { useNavigate } from "@tanstack/react-router";
import type { DownstreamService } from "@/types/api";
import {
  getAuthTypeLabel,
  isOidcService,
  SERVICE_CATEGORY_LABELS,
} from "@/lib/constants";
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
}

export function ServiceCard({
  service,
  onDelete,
  isDeleting,
}: ServiceCardProps) {
  const navigate = useNavigate();

  return (
    <Card
      className="cursor-pointer transition-colors hover:border-border/80"
      onClick={() =>
        void navigate({
          to: "/services/$serviceId",
          params: { serviceId: service.id },
        })
      }
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
            <Badge variant="outline">
              {SERVICE_CATEGORY_LABELS[service.service_category] ??
                service.service_category}
            </Badge>
          </div>
          <span className="text-xs text-muted-foreground">
            Created {formatDate(service.created_at)}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

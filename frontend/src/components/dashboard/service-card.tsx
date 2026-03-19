import { useNavigate } from "@tanstack/react-router";
import type { DownstreamService } from "@/types/api";
import {
  getAuthTypeLabel,
  isOidcService,
  SERVICE_TYPE_LABELS,
} from "@/lib/constants";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Lock, Trash2 } from "lucide-react";

interface ServiceCardProps {
  readonly service: DownstreamService;
  readonly onDelete: (id: string) => void;
  readonly isDeleting: boolean;
}

/* ── Service Card (VoidPortal) ── */
export function ServiceCard({
  service,
  onDelete,
  isDeleting,
}: ServiceCardProps) {
  const navigate = useNavigate();
  const secondaryLabel =
    service.service_type === "ssh"
      ? `${service.ssh_config?.host ?? "ssh"}:${String(service.ssh_config?.port ?? 22)}`
      : service.base_url;

  return (
    <div
      className="group relative flex cursor-pointer flex-col gap-4 rounded-[10px] border border-border bg-transparent p-6 transition-colors hover:border-border/80"
      onClick={() =>
        void navigate({
          to: "/services/$serviceId",
          params: { serviceId: service.id },
        })
      }
    >
      {/* Delete button (show on hover) */}
      <Button
        variant="ghost"
        size="icon"
        className="absolute right-2 top-2 h-7 w-7 opacity-0 transition-opacity group-hover:opacity-100 text-muted-foreground hover:text-destructive"
        onClick={(e) => {
          e.stopPropagation();
          onDelete(service.id);
        }}
        disabled={isDeleting}
      >
        <Trash2 className="h-3.5 w-3.5" />
        <span className="sr-only">Delete service</span>
      </Button>

      {/* Title + Badges row */}
      <div className="flex items-start justify-between gap-3">
        <h3 className="font-display text-lg font-normal text-foreground">
          {service.name}
        </h3>
        <div className="flex shrink-0 items-center gap-1.5">
          <Badge variant="secondary">
            {SERVICE_TYPE_LABELS[service.service_type] ?? service.service_type}
          </Badge>
          {service.service_type === "ssh" && (
            <Badge variant="outline">
              <Lock className="mr-1 h-2.5 w-2.5" />
              Private
            </Badge>
          )}
          {isOidcService(service) && (
            <Badge variant="accent">OIDC</Badge>
          )}
          {service.service_type === "http" && (
            <Badge variant="info">{getAuthTypeLabel(service)}</Badge>
          )}
        </div>
      </div>

      {/* Description (if exists) */}
      {service.description && (
        <p className="text-[13px] text-muted-foreground line-clamp-2">
          {service.description}
        </p>
      )}

      {/* Target */}
      <span className="text-xs text-text-tertiary">
        {secondaryLabel}
      </span>
    </div>
  );
}

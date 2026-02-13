import { useNavigate } from "@tanstack/react-router";
import type { DownstreamService } from "@/types/api";
import {
  getAuthTypeLabel,
  isOidcService,
} from "@/lib/constants";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Trash2 } from "lucide-react";

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
          {isOidcService(service) && (
            <Badge variant="accent">OIDC</Badge>
          )}
          <Badge variant="info">{getAuthTypeLabel(service)}</Badge>
        </div>
      </div>

      {/* Description (if exists) */}
      {service.description && (
        <p className="text-[13px] text-muted-foreground line-clamp-2">
          {service.description}
        </p>
      )}

      {/* Base URL */}
      <span className="text-xs text-text-tertiary">
        {service.base_url}
      </span>
    </div>
  );
}

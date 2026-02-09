import type { DownstreamService } from "@/types/api"
import { formatDate } from "@/lib/utils"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Server, Trash2 } from "lucide-react"

interface ServiceCardProps {
  readonly service: DownstreamService
  readonly onDelete: (id: string) => void
  readonly isDeleting: boolean
}

const AUTH_TYPE_LABELS: Record<string, string> = {
  api_key: "API Key",
  oauth2: "OAuth 2.0",
  basic: "Basic Auth",
  bearer: "Bearer Token",
}

export function ServiceCard({
  service,
  onDelete,
  isDeleting,
}: ServiceCardProps) {
  return (
    <Card className="transition-colors hover:border-border/80">
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
          onClick={() => onDelete(service.id)}
          disabled={isDeleting}
        >
          <Trash2 className="h-4 w-4" />
          <span className="sr-only">Delete service</span>
        </Button>
      </CardHeader>
      <CardContent>
        <div className="flex items-center justify-between">
          <Badge variant="secondary">
            {AUTH_TYPE_LABELS[service.auth_type] ?? service.auth_type}
          </Badge>
          <span className="text-xs text-muted-foreground">
            Created {formatDate(service.created_at)}
          </span>
        </div>
      </CardContent>
    </Card>
  )
}

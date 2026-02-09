import {
  useConnections,
  useServices,
  useConnectService,
  useDisconnectService,
} from "@/hooks/use-services"
import { formatDate } from "@/lib/utils"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import { Link2, Unlink, Server } from "lucide-react"
import { toast } from "sonner"

export function ConnectionGrid() {
  const { data: services, isLoading: servicesLoading } = useServices()
  const { data: connections, isLoading: connectionsLoading } = useConnections()
  const connectMutation = useConnectService()
  const disconnectMutation = useDisconnectService()

  const isLoading = servicesLoading || connectionsLoading

  async function handleConnect(serviceId: string) {
    try {
      await connectMutation.mutateAsync(serviceId)
      toast.success("Connected to service")
    } catch {
      toast.error("Failed to connect to service")
    }
  }

  async function handleDisconnect(serviceId: string) {
    try {
      await disconnectMutation.mutateAsync(serviceId)
      toast.success("Disconnected from service")
    } catch {
      toast.error("Failed to disconnect from service")
    }
  }

  if (isLoading) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {Array.from({ length: 6 }).map((_, i) => (
          <Skeleton key={`conn-skel-${String(i)}`} className="h-40 w-full" />
        ))}
      </div>
    )
  }

  if (!services || services.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <Server className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">
          No services available. Create a service first.
        </p>
      </div>
    )
  }

  const connectedIds = new Set(
    connections?.map((c) => c.service_id) ?? [],
  )

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {services.map((service) => {
        const isConnected = connectedIds.has(service.id)
        const connection = connections?.find(
          (c) => c.service_id === service.id,
        )

        return (
          <Card
            key={service.id}
            className={
              isConnected
                ? "border-primary/30 bg-primary/5"
                : "transition-colors hover:border-border/80"
            }
          >
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div
                    className={`flex h-10 w-10 items-center justify-center rounded-lg ${
                      isConnected
                        ? "bg-primary/20"
                        : "bg-muted"
                    }`}
                  >
                    <Server
                      className={`h-5 w-5 ${
                        isConnected ? "text-primary" : "text-muted-foreground"
                      }`}
                    />
                  </div>
                  <div>
                    <CardTitle className="text-base">{service.name}</CardTitle>
                    <CardDescription className="text-xs">
                      {service.base_url}
                    </CardDescription>
                  </div>
                </div>
                <Badge variant={isConnected ? "success" : "secondary"}>
                  {isConnected ? "Connected" : "Available"}
                </Badge>
              </div>
            </CardHeader>
            <CardContent>
              <div className="flex items-center justify-between">
                {isConnected && connection ? (
                  <>
                    <span className="text-xs text-muted-foreground">
                      Connected {formatDate(connection.connected_at)}
                    </span>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void handleDisconnect(service.id)}
                      disabled={disconnectMutation.isPending}
                    >
                      <Unlink className="mr-1.5 h-3 w-3" />
                      Disconnect
                    </Button>
                  </>
                ) : (
                  <>
                    <span className="text-xs text-muted-foreground">
                      Not connected
                    </span>
                    <Button
                      size="sm"
                      onClick={() => void handleConnect(service.id)}
                      disabled={connectMutation.isPending}
                    >
                      <Link2 className="mr-1.5 h-3 w-3" />
                      Connect
                    </Button>
                  </>
                )}
              </div>
            </CardContent>
          </Card>
        )
      })}
    </div>
  )
}

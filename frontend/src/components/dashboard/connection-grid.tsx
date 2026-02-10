import { useState } from "react";
import type { DownstreamService } from "@/types/api";
import {
  useConnections,
  useServices,
  useConnectService,
  useDisconnectService,
  useUpdateCredential,
} from "@/hooks/use-services";
import {
  isConnectable,
  isOidcService,
  getCredentialInputType,
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
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Link2, Unlink, Server, KeyRound } from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { CredentialDialog } from "./credential-dialog";

export function ConnectionGrid() {
  const { data: services, isLoading: servicesLoading } = useServices();
  const { data: connections, isLoading: connectionsLoading } = useConnections();
  const connectMutation = useConnectService();
  const disconnectMutation = useDisconnectService();
  const updateCredentialMutation = useUpdateCredential();

  const [credentialDialog, setCredentialDialog] = useState<{
    readonly service: DownstreamService;
    readonly mode: "connect" | "update";
  } | null>(null);

  const isLoading = servicesLoading || connectionsLoading;

  async function handleConnect(service: DownstreamService) {
    const inputType = getCredentialInputType(service);

    if (inputType.type === "none") {
      // Internal service: connect directly without credential
      try {
        await connectMutation.mutateAsync({ serviceId: service.id });
        toast.success("Connected to service");
      } catch (error) {
        if (error instanceof ApiError) {
          toast.error(error.message);
        } else {
          toast.error("Failed to connect to service");
        }
      }
    } else {
      // Connection service: open credential dialog
      setCredentialDialog({ service, mode: "connect" });
    }
  }

  async function handleCredentialSubmit(
    credential: string,
    label?: string,
  ) {
    if (!credentialDialog) return;
    const { service, mode } = credentialDialog;

    try {
      if (mode === "connect") {
        await connectMutation.mutateAsync({
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success("Connected to service");
      } else {
        await updateCredentialMutation.mutateAsync({
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success("Credential updated");
      }
      setCredentialDialog(null);
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error(
          mode === "connect"
            ? "Failed to connect to service"
            : "Failed to update credential",
        );
      }
    }
  }

  async function handleDisconnect(serviceId: string) {
    try {
      await disconnectMutation.mutateAsync(serviceId);
      toast.success("Disconnected from service");
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to disconnect from service");
      }
    }
  }

  if (isLoading) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {Array.from({ length: 6 }).map((_, i) => (
          <Skeleton key={`conn-skel-${String(i)}`} className="h-40 w-full" />
        ))}
      </div>
    );
  }

  // Filter: only connectable services (exclude providers and OIDC)
  const connectableServices = services?.filter(
    (s) => isConnectable(s) && !isOidcService(s),
  ) ?? [];

  if (connectableServices.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <Server className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">
          No connectable services available. Create a service first.
        </p>
      </div>
    );
  }

  const connectedIds = new Set(connections?.map((c) => c.service_id) ?? []);

  return (
    <>
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {connectableServices.map((service) => {
          const isConnected = connectedIds.has(service.id);
          const connection = connections?.find(
            (c) => c.service_id === service.id,
          );

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
                        isConnected ? "bg-primary/20" : "bg-muted"
                      }`}
                    >
                      <Server
                        className={`h-5 w-5 ${
                          isConnected
                            ? "text-primary"
                            : "text-muted-foreground"
                        }`}
                      />
                    </div>
                    <div>
                      <CardTitle className="text-base">
                        {service.name}
                      </CardTitle>
                      <CardDescription className="text-xs">
                        {service.base_url}
                      </CardDescription>
                    </div>
                  </div>
                  <div className="flex flex-col items-end gap-1">
                    <Badge variant={isConnected ? "success" : "secondary"}>
                      {isConnected ? "Connected" : "Available"}
                    </Badge>
                    <Badge variant="outline" className="text-[10px]">
                      {SERVICE_CATEGORY_LABELS[service.service_category] ??
                        service.service_category}
                    </Badge>
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                <div className="flex items-center justify-between">
                  {isConnected && connection ? (
                    <>
                      <div className="flex flex-col gap-0.5">
                        <span className="text-xs text-muted-foreground">
                          Connected {formatDate(connection.connected_at)}
                        </span>
                        {connection.has_credential &&
                          connection.credential_label && (
                            <span className="text-xs text-muted-foreground/70">
                              {connection.credential_label}
                            </span>
                          )}
                        {service.requires_user_credential &&
                          !connection.has_credential && (
                            <span className="text-xs text-destructive">
                              Credential missing
                            </span>
                          )}
                      </div>
                      <div className="flex gap-1.5">
                        {service.requires_user_credential && (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() =>
                              setCredentialDialog({
                                service,
                                mode: "update",
                              })
                            }
                            disabled={updateCredentialMutation.isPending}
                          >
                            <KeyRound className="mr-1.5 h-3 w-3" />
                            Update Key
                          </Button>
                        )}
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => void handleDisconnect(service.id)}
                          disabled={disconnectMutation.isPending}
                        >
                          <Unlink className="mr-1.5 h-3 w-3" />
                          Disconnect
                        </Button>
                      </div>
                    </>
                  ) : (
                    <>
                      <span className="text-xs text-muted-foreground">
                        Not connected
                      </span>
                      <Button
                        size="sm"
                        onClick={() => void handleConnect(service)}
                        disabled={connectMutation.isPending}
                      >
                        <Link2 className="mr-1.5 h-3 w-3" />
                        {service.requires_user_credential
                          ? "Connect"
                          : "Enable"}
                      </Button>
                    </>
                  )}
                </div>
              </CardContent>
            </Card>
          );
        })}
      </div>

      {credentialDialog !== null && (
        <CredentialDialog
          service={credentialDialog.service}
          mode={credentialDialog.mode}
          onSubmit={(credential, label) =>
            void handleCredentialSubmit(credential, label)
          }
          onCancel={() => setCredentialDialog(null)}
          isPending={
            connectMutation.isPending || updateCredentialMutation.isPending
          }
        />
      )}
    </>
  );
}

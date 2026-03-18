import { useState } from "react";
import {
  useSaConnections,
  useConnectSaService,
  useUpdateSaConnectionCredential,
  useDisconnectSaService,
} from "@/hooks/use-service-accounts";
import { useServices } from "@/hooks/use-services";
import { formatDate } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { CredentialDialog } from "@/components/dashboard/credential-dialog";
import type { DownstreamService } from "@/types/api";
import { DetailSection } from "@/components/shared/detail-section";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Plug, Unlink, KeyRound } from "lucide-react";
import { toast } from "sonner";

interface SaConnectedServicesProps {
  readonly saId: string;
}

export function SaConnectedServices({ saId }: SaConnectedServicesProps) {
  const { data: saConnections, isLoading: connectionsLoading } = useSaConnections(saId);
  const { data: allServices } = useServices();
  const connectServiceMutation = useConnectSaService();
  const updateConnectionCredentialMutation = useUpdateSaConnectionCredential();
  const disconnectServiceMutation = useDisconnectSaService();

  const connectedServiceIds = new Set(saConnections?.map((c) => c.service_id) ?? []);
  const availableServices = (allServices ?? []).filter(
    (s) =>
      s.is_active &&
      s.service_type === "http" &&
      s.service_category !== "provider" &&
      !connectedServiceIds.has(s.id),
  );

  const [serviceCredentialDialog, setServiceCredentialDialog] = useState<{
    readonly service: DownstreamService;
    readonly mode: "connect" | "update";
  } | null>(null);

  function handleConnectService(service: DownstreamService) {
    if (service.requires_user_credential) {
      setServiceCredentialDialog({ service, mode: "connect" });
    } else {
      void handleConnectServiceDirect(service.id);
    }
  }

  async function handleConnectServiceDirect(serviceId: string) {
    try {
      await connectServiceMutation.mutateAsync({ saId, serviceId });
      toast.success("Service connected");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to connect service");
      }
    }
  }

  async function handleServiceCredentialSubmit(
    credential: string,
    label?: string,
  ) {
    if (!serviceCredentialDialog) return;
    const { service, mode } = serviceCredentialDialog;

    try {
      if (mode === "connect") {
        await connectServiceMutation.mutateAsync({
          saId,
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success(`Connected to ${service.name}`);
      } else {
        await updateConnectionCredentialMutation.mutateAsync({
          saId,
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success("Credential updated");
      }
      setServiceCredentialDialog(null);
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(
          mode === "connect"
            ? "Failed to connect service"
            : "Failed to update credential",
        );
      }
    }
  }

  function handleUpdateServiceCredential(serviceId: string) {
    const service = allServices?.find((s) => s.id === serviceId);
    if (service) {
      setServiceCredentialDialog({ service, mode: "update" });
    }
  }

  async function handleDisconnectService(serviceId: string) {
    try {
      await disconnectServiceMutation.mutateAsync({ saId, serviceId });
      toast.success("Service disconnected");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to disconnect service");
      }
    }
  }

  return (
    <>
      <DetailSection title="Connected Services">
        {connectionsLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : saConnections && saConnections.length > 0 ? (
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>Category</TableHead>
                  <TableHead>Auth Type</TableHead>
                  <TableHead>Credential</TableHead>
                  <TableHead>Label</TableHead>
                  <TableHead>Connected</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {saConnections.map((conn) => (
                  <TableRow key={conn.service_id}>
                    <TableCell className="font-medium">{conn.service_name}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{conn.service_category}</Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {conn.auth_type ?? "-"}
                    </TableCell>
                    <TableCell>
                      <Badge variant={conn.has_credential ? "success" : "secondary"}>
                        {conn.has_credential ? "Stored" : "None"}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {conn.credential_label ?? "-"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(conn.connected_at)}
                    </TableCell>
                    <TableCell>
                      <div className="flex gap-1">
                        {conn.has_credential && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleUpdateServiceCredential(conn.service_id)}
                            disabled={updateConnectionCredentialMutation.isPending}
                          >
                            <KeyRound className="mr-1 h-3 w-3" />
                            Update
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void handleDisconnectService(conn.service_id)}
                          disabled={disconnectServiceMutation.isPending}
                        >
                          <Unlink className="mr-1 h-3 w-3" />
                          Disconnect
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No services connected to this service account.
          </p>
        )}

        {availableServices.length > 0 && (
          <div className="mt-3">
            <ConnectServiceDropdown
              services={availableServices}
              onSelect={handleConnectService}
            />
          </div>
        )}
      </DetailSection>

      {serviceCredentialDialog !== null && (
        <CredentialDialog
          service={serviceCredentialDialog.service}
          mode={serviceCredentialDialog.mode}
          onSubmit={(credential, label) =>
            void handleServiceCredentialSubmit(credential, label)
          }
          onCancel={() => setServiceCredentialDialog(null)}
          isPending={
            connectServiceMutation.isPending ||
            updateConnectionCredentialMutation.isPending
          }
        />
      )}
    </>
  );
}

function ConnectServiceDropdown({
  services,
  onSelect,
}: {
  readonly services: readonly DownstreamService[];
  readonly onSelect: (service: DownstreamService) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <Plug className="mr-1 h-3 w-3" />
          Connect Service
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        {services.map((s) => (
          <DropdownMenuItem key={s.id} onClick={() => onSelect(s)}>
            <span>{s.name}</span>
            {s.requires_user_credential && (
              <KeyRound className="ml-1 h-3 w-3 text-muted-foreground" />
            )}
            <Badge variant="outline" className="ml-auto text-xs">
              {s.service_category}
            </Badge>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

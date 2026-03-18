import { useEffect, useMemo } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { ApiError } from "@/lib/api-client";
import { usePublicConfig } from "@/hooks/use-public-config";
import {
  useDeleteSshServiceConfig,
  useSshServiceConfig,
  useUpsertSshServiceConfig,
} from "@/hooks/use-services";
import {
  sshServiceConfigSchema,
  type SshServiceConfigFormData,
} from "@/schemas/services";
import { CopyableField } from "@/components/shared/copyable-field";
import { DetailRow } from "@/components/shared/detail-row";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "sonner";

interface SshAccessPanelProps {
  readonly serviceId: string;
  readonly serviceSlug: string;
}

const DEFAULT_VALUES: SshServiceConfigFormData = {
  host: "",
  port: "22",
  certificate_auth_enabled: false,
  certificate_ttl_minutes: "30",
  allowed_principals: "",
};

function parseAllowedPrincipals(input: string): string[] {
  return input
    .split(/[\n,]/)
    .map((principal) => principal.trim())
    .filter(Boolean);
}

function deriveNyxidBaseUrl(nodeWsUrl?: string): string {
  if (nodeWsUrl) {
    try {
      const parsed = new URL(nodeWsUrl);
      parsed.protocol = parsed.protocol === "wss:" ? "https:" : "http:";
      parsed.pathname = "";
      parsed.search = "";
      parsed.hash = "";
      return parsed.toString().replace(/\/$/, "");
    } catch {
      // Fall through to the browser origin.
    }
  }

  if (typeof window !== "undefined") {
    return window.location.origin;
  }

  return "https://auth.example.com";
}

export function SshAccessPanel({
  serviceId,
  serviceSlug,
}: SshAccessPanelProps) {
  const { data: publicConfig } = usePublicConfig();
  const {
    data: sshConfig,
    isLoading,
    error,
  } = useSshServiceConfig(serviceId);
  const upsertMutation = useUpsertSshServiceConfig();
  const deleteMutation = useDeleteSshServiceConfig();

  const form = useForm<SshServiceConfigFormData>({
    resolver: zodResolver(sshServiceConfigSchema),
    defaultValues: DEFAULT_VALUES,
  });

  useEffect(() => {
    if (!sshConfig) {
      form.reset(DEFAULT_VALUES);
      return;
    }

    form.reset({
      host: sshConfig.host,
      port: String(sshConfig.port),
      certificate_auth_enabled: sshConfig.certificate_auth_enabled,
      certificate_ttl_minutes: String(sshConfig.certificate_ttl_minutes),
      allowed_principals: sshConfig.allowed_principals.join(", "),
    });
  }, [form, sshConfig]);

  const nyxidBaseUrl = useMemo(
    () => deriveNyxidBaseUrl(publicConfig?.node_ws_url),
    [publicConfig?.node_ws_url],
  );

  const primaryPrincipal = sshConfig?.allowed_principals[0] ?? "ubuntu";
  const certificateFile = `~/.ssh/nyxid/${serviceSlug}-cert.pub`;
  const caPublicKeyFile = `~/.ssh/nyxid/${serviceSlug}-ca.pub`;
  const installCommand = "cargo install --path backend";
  const tokenCommand = 'export NYXID_ACCESS_TOKEN="<paste-access-token>"';
  const transportCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId}' <ssh-user>@ssh.invalid`;
  const certificateCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId} --issue-certificate --public-key-file ~/.ssh/id_ed25519.pub --principal ${primaryPrincipal} --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}' -o CertificateFile=${certificateFile} -o IdentityFile=~/.ssh/id_ed25519 ${primaryPrincipal}@ssh.invalid`;
  const configCommand = `nyxid ssh config --host-alias ${serviceSlug} --base-url ${nyxidBaseUrl} --service-id ${serviceId} --principal ${primaryPrincipal} --identity-file ~/.ssh/id_ed25519 --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}`;

  async function onSubmit(data: SshServiceConfigFormData) {
    try {
      await upsertMutation.mutateAsync({
        serviceId,
        data: {
          host: data.host.trim(),
          port: Number(data.port),
          certificate_auth_enabled: data.certificate_auth_enabled,
          certificate_ttl_minutes: Number(data.certificate_ttl_minutes),
          allowed_principals: parseAllowedPrincipals(data.allowed_principals),
        },
      });
      toast.success(
        sshConfig ? "SSH access updated" : "SSH access enabled",
      );
      form.clearErrors("root");
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to save SSH access");
      }
    }
  }

  async function handleDisable() {
    try {
      await deleteMutation.mutateAsync(serviceId);
      toast.success("SSH access disabled");
      form.reset(DEFAULT_VALUES);
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to disable SSH access");
      }
    }
  }

  return (
    <div className="rounded-xl border border-border p-4">
      <div className="space-y-1">
        <h3 className="font-display text-sm font-semibold">SSH Access</h3>
        <p className="text-xs text-muted-foreground">
          Configure the SSH tunnel target and copy the built-in `nyxid ssh`
          helper commands for terminal use.
        </p>
      </div>

      {isLoading ? (
        <div className="mt-4 space-y-3">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-40 w-full" />
        </div>
      ) : error ? (
        <div className="mt-4 rounded-[10px] border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          {error instanceof ApiError ? error.message : "Failed to load SSH access"}
        </div>
      ) : (
        <div className="mt-4 space-y-4">
          {sshConfig ? (
            <div className="space-y-3">
              <DetailRow
                label="Tunnel Status"
                value={sshConfig.enabled ? "Enabled" : "Disabled"}
                badge
                badgeVariant={sshConfig.enabled ? "success" : "secondary"}
              />
              <DetailRow
                label="Target"
                value={`${sshConfig.host}:${String(sshConfig.port)}`}
                copyable
                mono
              />
              <DetailRow
                label="Certificate Auth"
                value={sshConfig.certificate_auth_enabled ? "Enabled" : "Transport only"}
                badge
                badgeVariant={sshConfig.certificate_auth_enabled ? "success" : "secondary"}
              />
              {sshConfig.certificate_auth_enabled && (
                <>
                  <DetailRow
                    label="Certificate TTL"
                    value={`${String(sshConfig.certificate_ttl_minutes)} minutes`}
                  />
                  <DetailRow
                    label="Allowed Principals"
                    value={sshConfig.allowed_principals.join(", ")}
                    copyable
                  />
                </>
              )}
              {sshConfig.ca_public_key && (
                <CopyableField
                  label="SSH CA Public Key"
                  value={sshConfig.ca_public_key}
                  size="sm"
                />
              )}
            </div>
          ) : (
            <div className="rounded-[10px] border border-dashed border-border p-3 text-sm text-muted-foreground">
              SSH tunneling is not configured for this service yet.
            </div>
          )}

          {sshConfig && (
            <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
              <div className="space-y-1">
                <h4 className="text-sm font-semibold">Helper Commands</h4>
                <p className="text-xs text-muted-foreground">
                  Install the helper once, export a NyxID access token, then
                  paste one of these commands into your terminal.
                </p>
              </div>
              <CopyableField label="Install helper" value={installCommand} size="sm" />
              <CopyableField label="Export access token" value={tokenCommand} size="sm" />
              <CopyableField
                label="One-off SSH command"
                value={
                  sshConfig.certificate_auth_enabled
                    ? certificateCommand
                    : transportCommand
                }
                size="sm"
              />
              {sshConfig.certificate_auth_enabled && (
                <CopyableField
                  label="Generate SSH config stanza"
                  value={configCommand}
                  size="sm"
                />
              )}
            </div>
          )}

          <Form {...form}>
            <form
              onSubmit={form.handleSubmit(onSubmit)}
              className="space-y-4 rounded-[10px] border border-border p-3"
            >
              {form.formState.errors.root?.message && (
                <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                  {form.formState.errors.root.message}
                </div>
              )}

              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="host"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>SSH Host</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="ssh.internal.example"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="port"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>SSH Port</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          min={1}
                          max={65535}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <div className="flex items-center justify-between rounded-[10px] border border-border p-3">
                <Label
                  htmlFor={`ssh-cert-auth-${serviceId}`}
                  className="text-sm font-normal"
                >
                  Enable short-lived SSH certificates
                </Label>
                <Switch
                  id={`ssh-cert-auth-${serviceId}`}
                  checked={form.watch("certificate_auth_enabled")}
                  onCheckedChange={(checked) =>
                    form.setValue("certificate_auth_enabled", checked)
                  }
                />
              </div>

              {form.watch("certificate_auth_enabled") && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="certificate_ttl_minutes"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Certificate TTL (minutes)</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={15}
                            max={60}
                            {...field}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="allowed_principals"
                    render={({ field }) => (
                      <FormItem className="sm:col-span-1">
                        <FormLabel>Allowed Principals</FormLabel>
                        <FormControl>
                          <Input
                            placeholder="ubuntu, deploy"
                            {...field}
                          />
                        </FormControl>
                        <p className="text-xs text-muted-foreground">
                          Comma-separated SSH usernames NyxID is allowed to sign.
                        </p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
              )}

              <div className="flex flex-wrap items-center gap-3">
                <Button type="submit" isLoading={upsertMutation.isPending}>
                  {sshConfig ? "Save SSH access" : "Enable SSH access"}
                </Button>
                {sshConfig && (
                  <Button
                    type="button"
                    variant="destructive"
                    isLoading={deleteMutation.isPending}
                    onClick={() => void handleDisable()}
                  >
                    Disable SSH access
                  </Button>
                )}
              </div>
            </form>
          </Form>
        </div>
      )}
    </div>
  );
}

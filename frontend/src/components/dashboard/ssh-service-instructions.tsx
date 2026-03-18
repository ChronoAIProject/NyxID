import { usePublicConfig } from "@/hooks/use-public-config";
import type { SshServiceConfig } from "@/types/api";
import { CopyableField } from "@/components/shared/copyable-field";
import { deriveNyxidBaseUrl } from "@/lib/ssh";

interface SshServiceInstructionsProps {
  readonly serviceId: string;
  readonly serviceSlug: string;
  readonly sshConfig: SshServiceConfig;
}

export function SshServiceInstructions({
  serviceId,
  serviceSlug,
  sshConfig,
}: SshServiceInstructionsProps) {
  const { data: publicConfig } = usePublicConfig();

  const nyxidBaseUrl = deriveNyxidBaseUrl(publicConfig?.node_ws_url);

  const primaryPrincipal = sshConfig.allowed_principals[0] ?? "ubuntu";
  const certificateFile = `~/.ssh/nyxid/${serviceSlug}-cert.pub`;
  const caPublicKeyFile = `~/.ssh/nyxid/${serviceSlug}-ca.pub`;
  const installCommand = "cargo install --path backend";
  const tokenCommand = 'export NYXID_ACCESS_TOKEN="<paste-access-token>"';
  const transportCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId}' <ssh-user>@ssh.invalid`;
  const certificateCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId} --issue-certificate --public-key-file ~/.ssh/id_ed25519.pub --principal ${primaryPrincipal} --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}' -o CertificateFile=${certificateFile} -o IdentityFile=~/.ssh/id_ed25519 ${primaryPrincipal}@ssh.invalid`;
  const configCommand = `nyxid ssh config --host-alias ${serviceSlug} --base-url ${nyxidBaseUrl} --service-id ${serviceId} --principal ${primaryPrincipal} --identity-file ~/.ssh/id_ed25519 --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}`;

  return (
    <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
      <div className="space-y-1">
        <h4 className="text-sm font-semibold">NyxID SSH Helper</h4>
        <p className="text-xs text-muted-foreground">
          Install the helper once, export a NyxID access token, then paste one
          of these commands into your terminal.
        </p>
      </div>
      <CopyableField label="Install helper" value={installCommand} size="sm" />
      <CopyableField
        label="Export access token"
        value={tokenCommand}
        size="sm"
      />
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
  );
}

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
  const loginCommand = `nyxid login --base-url ${nyxidBaseUrl}`;
  const apiKeyCommand = 'export NYXID_ACCESS_TOKEN="nyx_..."';
  const transportCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId}' <ssh-user>@ssh.invalid`;
  const certificateCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId} --issue-certificate --public-key-file ~/.ssh/id_ed25519.pub --principal ${primaryPrincipal} --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}' -o CertificateFile=${certificateFile} -o IdentityFile=~/.ssh/id_ed25519 ${primaryPrincipal}@ssh.invalid`;
  const configCommand = `nyxid ssh config --host-alias ${serviceSlug} --base-url ${nyxidBaseUrl} --service-id ${serviceId} --principal ${primaryPrincipal} --identity-file ~/.ssh/id_ed25519 --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}`;

  return (
    <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
      <div className="space-y-1">
        <h4 className="text-sm font-semibold">NyxID SSH Helper</h4>
        <p className="text-xs text-muted-foreground">
          Install the helper once, authenticate, then paste a connect command
          into your terminal.
        </p>
      </div>
      <CopyableField label="1. Install helper" value={installCommand} size="sm" />

      <div className="space-y-1">
        <p className="text-xs font-medium text-muted-foreground">
          2. Authenticate (choose one)
        </p>
      </div>
      <CopyableField
        label="Option A: Login (recommended)"
        value={loginCommand}
        size="sm"
      />
      <CopyableField
        label="Option B: Use an API key"
        value={apiKeyCommand}
        size="sm"
      />

      <CopyableField
        label="3. Connect"
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

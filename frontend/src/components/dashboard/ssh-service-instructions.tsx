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
  const sshTarget = `${primaryPrincipal}@${serviceSlug}`;
  const keyPlaceholder = "<your-key>";
  const keyHint = "~/.ssh/id_ed25519, ~/.ssh/id_rsa, etc.";

  const installCommand = "cargo install --path backend";
  const loginCommand = `nyxid login --base-url ${nyxidBaseUrl}`;
  const apiKeyCommand = 'export NYXID_ACCESS_TOKEN="nyx_..."';

  const transportCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId}' ${sshTarget}`;
  const certificateCommand = `ssh -o ProxyCommand='nyxid ssh proxy --base-url ${nyxidBaseUrl} --service-id ${serviceId} --issue-certificate --public-key-file ${keyPlaceholder}.pub --principal ${primaryPrincipal} --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}' -o CertificateFile=${certificateFile} -o IdentityFile=${keyPlaceholder} ${sshTarget}`;
  const configCommand = `nyxid ssh config --host-alias ${serviceSlug} --base-url ${nyxidBaseUrl} --service-id ${serviceId} --principal ${primaryPrincipal} --identity-file ${keyPlaceholder} --certificate-file ${certificateFile} --ca-public-key-file ${caPublicKeyFile}`;

  // Target machine setup commands
  const caPublicKey = sshConfig.ca_public_key ?? "<CA public key from above>";
  const trustedCaCommand = `echo '${caPublicKey}' | sudo tee /etc/ssh/nyxid_ca.pub`;
  const sshdConfigCommand = `echo 'TrustedUserCAKeys /etc/ssh/nyxid_ca.pub' | sudo tee -a /etc/ssh/sshd_config && echo 'AuthorizedPrincipalsFile /etc/ssh/auth_principals/%u' | sudo tee -a /etc/ssh/sshd_config`;
  const principalsCommand = [
    "sudo mkdir -p /etc/ssh/auth_principals",
    ...(sshConfig.allowed_principals.length > 0
      ? sshConfig.allowed_principals.map((p) => `echo '${p}' | sudo tee /etc/ssh/auth_principals/${p}`)
      : [`echo '${primaryPrincipal}' | sudo tee /etc/ssh/auth_principals/${primaryPrincipal}`]),
  ].join(" && ");
  const restartSshdLinux = "sudo systemctl restart sshd";
  const restartSshdMac = "sudo launchctl kickstart -k system/com.openssh.sshd";

  // Node-agent setup (for targets not directly reachable from NyxID server)
  const nodeRegisterCommand = `nyxid-node register --token <registration-token> --url ${nyxidBaseUrl.replace("http://", "ws://").replace("https://", "wss://")}/api/v1/nodes/ws`;
  const nodeStartCommand = "nyxid-node start";

  return (
    <div className="space-y-6">
      {/* Client Setup */}
      <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
        <div className="space-y-1">
          <h4 className="text-sm font-semibold">Client Setup</h4>
          <p className="text-xs text-muted-foreground">
            Install the NyxID CLI on your local machine and authenticate.
          </p>
        </div>
        <CopyableField label="1. Install CLI" value={installCommand} size="sm" />
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground">
            2. Authenticate (choose one)
          </p>
        </div>
        <CopyableField label="Option A: Browser login (recommended)" value={loginCommand} size="sm" />
        <CopyableField label="Option B: API key" value={apiKeyCommand} size="sm" />
        <CopyableField
          label="3. Connect"
          value={sshConfig.certificate_auth_enabled ? certificateCommand : transportCommand}
          size="sm"
        />
        {sshConfig.certificate_auth_enabled && (
          <>
            <p className="text-xs text-muted-foreground">
              Replace <code className="rounded bg-muted px-1 text-[10px]">{keyPlaceholder}</code> with
              your SSH private key path ({keyHint}).
            </p>
            <CopyableField label="Optional: Generate SSH config stanza" value={configCommand} size="sm" />
          </>
        )}
      </div>

      {/* Target Machine Setup (passwordless) */}
      {sshConfig.certificate_auth_enabled && (
        <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
          <div className="space-y-1">
            <h4 className="text-sm font-semibold">Target Machine Setup (Passwordless Login)</h4>
            <p className="text-xs text-muted-foreground">
              Run these commands on the SSH target machine to trust NyxID certificates.
              Each principal maps to a Unix user -- only users listed in the
              authorized principals file can log in via that principal.
            </p>
          </div>
          <CopyableField
            label="1. Install NyxID CA public key"
            value={trustedCaCommand}
            size="sm"
          />
          <CopyableField
            label="2. Configure sshd to trust NyxID CA"
            value={sshdConfigCommand}
            size="sm"
          />
          <CopyableField
            label="3. Create authorized principals for each user"
            value={principalsCommand}
            size="sm"
          />
          <CopyableField
            label="4. Restart SSH daemon (Linux)"
            value={restartSshdLinux}
            size="sm"
          />
          <CopyableField
            label="4. Restart SSH daemon (macOS)"
            value={restartSshdMac}
            size="sm"
          />
          <p className="text-xs text-muted-foreground">
            macOS: ensure Remote Login is enabled in System Settings &gt; General &gt;
            Sharing (&nbsp;or{" "}
            <code className="rounded bg-muted px-1 text-[10px]">sudo systemsetup -setremotelogin on</code>
            ). The sshd_config path is{" "}
            <code className="rounded bg-muted px-1 text-[10px]">/etc/ssh/sshd_config</code>{" "}
            (same as Linux). On recent macOS, SIP may restrict direct edits to /etc/ssh/ --
            use <code className="rounded bg-muted px-1 text-[10px]">sudo</code> to write
            config files. Ensure CA key file permissions are{" "}
            <code className="rounded bg-muted px-1 text-[10px]">644</code> and
            auth_principals directories are{" "}
            <code className="rounded bg-muted px-1 text-[10px]">755</code>.
          </p>
          <p className="text-xs text-muted-foreground">
            How it works: NyxID signs short-lived certificates with a specific principal
            (e.g., &quot;{primaryPrincipal}&quot;). The target machine checks that the
            certificate is signed by the trusted CA AND that the principal is listed in{" "}
            <code className="rounded bg-muted px-1 text-[10px]">/etc/ssh/auth_principals/{primaryPrincipal}</code>.
            This means even if someone has a valid NyxID certificate, they can only access
            accounts whose principals file includes their certificate&apos;s principal.
            NyxID verifies the user&apos;s identity (via JWT or API key) before signing
            any certificate, and only signs principals from the service&apos;s allowed list.
          </p>
        </div>
      )}

      {/* Node-Agent Setup (for unreachable targets) */}
      <div className="space-y-3 rounded-[10px] border border-border bg-muted/20 p-3">
        <div className="space-y-1">
          <h4 className="text-sm font-semibold">Node Agent (Remote Targets)</h4>
          <p className="text-xs text-muted-foreground">
            If the SSH target is behind a firewall or not directly reachable from the
            NyxID server, deploy a node agent on a machine that CAN reach the target.
            The node agent establishes an outbound WebSocket connection to NyxID and
            tunnels SSH traffic through it -- no inbound ports required.
          </p>
        </div>
        <CopyableField
          label="1. On the node machine: register"
          value={nodeRegisterCommand}
          size="sm"
        />
        <CopyableField
          label="2. Start the agent"
          value={nodeStartCommand}
          size="sm"
        />
        <p className="text-xs text-muted-foreground">
          After registering, bind this SSH service to the node via the Nodes page.
          When a user connects, NyxID routes the SSH tunnel through the node agent
          instead of connecting directly. The node agent handles the TCP connection
          to the SSH target on its local network.
        </p>
      </div>
    </div>
  );
}

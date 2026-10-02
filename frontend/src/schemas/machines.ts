import { z } from "zod";

export const machineChoicesSchema = z.object({
  owner_id: z.string().uuid().nullable().optional(),
  name: z
    .string()
    .min(1)
    .max(64)
    .regex(/^[a-z0-9-]+$/, "Use lowercase letters, numbers and hyphens"),
  where: z.enum(["this_computer", "vm", "docker"]),
  capabilities: z
    .array(z.enum(["shell", "files", "computer"]))
    .min(1)
    .max(3),
  grant_to: z.string().nullable(),
  automatic_updates: z.boolean().nullable().optional(),
});
export type MachineChoices = z.infer<typeof machineChoicesSchema>;
export interface MachineProfile {
  version: number;
  shell: boolean;
  files: boolean;
  computer: boolean;
  os: string;
  arch: string;
  roots: string[];
  computer_mode: "standard" | "unrestricted";
  cua_version: string | null;
  computer_tools: string[];
  computer_ready: boolean;
  computer_permissions?: {
    screen_recording: boolean | null;
    accessibility: boolean | null;
  } | null;
  browser_isolated: boolean;
  commands_isolated?: boolean | null;
  saved_login_ready: boolean;
  installation?: "container" | "native";
  updater_ready?: boolean;
}
export interface MachineSetup {
  id: string;
  choices: MachineChoices;
  status: string;
  hostname: string | null;
  os: string | null;
  ip: string | null;
  conversation_id: string | null;
  expires_at: string;
  machine: MachineProfile | null;
}
export const MACHINE_SAFETY =
  "Recommended: use the machine container or a VM set up with --separate-users. Agents act with that OS user's full access, and prompt injection is possible. Workspace roots constrain file tools and working directories; shell commands can access anything that user can.";
export const SINGLE_USER_SHELL_WARNING =
  "Not isolated: agent commands can read this node's stored credentials, signing secret and node token, including its config and local credential store. File-tool workspace limits do not constrain the shell. You may proceed; prefer the machine container or a VM set up with --separate-users.";
export const SINGLE_USER_WARNING =
  "Agent commands run as the same user as the browser on this machine. A misbehaving or prompt-injected agent could read what is typed. Use the machine container or a VM with separate browser and agent users for better isolation.";

/** Shell quoting is applied to every value from a page, API or runtime config. */
export function shellQuote(value: string): string {
  if (value.includes("\0")) throw new Error("Invalid command argument");
  return `'${value.replaceAll("'", "'\\''")}'`;
}
export function machineSetupCommand(
  choices: MachineChoices,
  token: string,
  wsUrl: string,
  version?: string,
  updaterImage?: string,
): string {
  if (!/^nyx_nreg_[a-f0-9]{64}$/.test(token))
    throw new Error("Invalid setup credential");
  const args = choices.capabilities
    .map((capability) => `--${capability}`)
    .join(" ");
  if (choices.where === "docker") {
    if (!version || !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version))
      throw new Error(
        "Server release version is unavailable. Reload before creating the Docker command.",
      );
    const profileUrl = new URL("/machine-seccomp.json", window.location.origin)
      .href;
    return `(nyx_machine_dir=$(mktemp -d) && trap 'rm -rf "$nyx_machine_dir"' EXIT && curl -fsSL ${shellQuote(profileUrl)} -o "$nyx_machine_dir/seccomp.json" && docker run -d --security-opt "seccomp=$nyx_machine_dir/seccomp.json" --name ${shellQuote(choices.name)} --restart unless-stopped --shm-size=1g --label ${shellQuote(`dev.nyxid.machine=${choices.name}`)} -v ${shellQuote(`${choices.name}-nyxid-update:/var/lib/nyxid-machine-update`)} -v ${shellQuote(`${choices.name}-identity:/var/lib/nyxid-machine`)} -v ${shellQuote(`${choices.name}-workspace:/workspace`)} -e ${shellQuote(`NYXID_NODE_TOKEN=${token}`)} -e ${shellQuote(`NYXID_NODE_URL=${wsUrl}`)} ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:${version} ${args} && ${machineCompanionCommand(choices.name, version, updaterImage)})`;
  }
  const installer =
    "https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh";
  return `(command -v nyxid >/dev/null 2>&1 || bash -c "$(curl -fsSL ${installer})") && PATH="$HOME/.local/bin:$PATH" nyxid node setup --token ${shellQuote(token)} --url ${shellQuote(wsUrl)} ${args}`;
}

export interface MachineUpdateStatus {
  node_id: string;
  current_version: string;
  target_version: string;
  update_available: boolean;
  updater_ready: boolean;
  updater?: {
    version: string;
    digest: string | null;
    target_version: string | null;
    phase: "current" | "pending" | "failed" | "legacy";
    code: string | null;
  } | null;
  updater_guidance?: string | null;
  installation: "container" | "native";
  automatic: boolean;
  phase: string;
  code: string | null;
  guidance?: string | null;
  settings_path: string;
}

function validateUpdate(name: string, version: string) {
  if (
    !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name) ||
    !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)
  )
    throw new Error(
      "Provide a valid Docker container name and server release.",
    );
}

function verifiedUpdaterImage(image?: string) {
  if (!image || !/^ghcr\.io\/chronoaiproject\/nyxid\/nyxid-machine-updater@sha256:[a-f0-9]{64}$/.test(image))
    throw new Error("Verifying the updater image, try again shortly.");
  return image;
}

export function machineCompanionCommand(name: string, version: string, image?: string) {
  validateUpdate(name, version);
  return `docker run -d --name ${shellQuote(`${name}-updater`)} --restart unless-stopped --label ${shellQuote(`dev.nyxid.machine.updater=${name}`)} --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop=ALL --security-opt=no-new-privileges --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock --mount ${shellQuote(`type=volume,src=${name}-nyxid-update,dst=/var/lib/nyxid-machine-update`)} ${verifiedUpdaterImage(image)} watch ${shellQuote(name)}`;
}

export function machineCompanionReplacementCommand(name: string, version: string, image?: string) {
  const companion = machineCompanionCommand(name, version, image);
  return `docker rm -f ${shellQuote(`${name}-updater`)} && ${companion}`;
}

export function machineMigrationCommand(name: string, version: string, image?: string) {
  validateUpdate(name, version);
  return `docker run --rm --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop=ALL --security-opt=no-new-privileges --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock --mount ${shellQuote(`type=volume,src=${name}-nyxid-update,dst=/var/lib/nyxid-machine-update`)} ${verifiedUpdaterImage(image)} bootstrap ${shellQuote(name)} ${shellQuote(version)}`;
}

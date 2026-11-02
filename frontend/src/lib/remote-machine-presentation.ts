import type { MachineProfile, RemoteMachine } from "@/schemas/machines";
/** Display adapter only; never used as execution authority or capability evidence. */
export function remoteMachineReportedProfile(
  presentation: RemoteMachine["presentation"],
): MachineProfile | undefined {
  if (
    !presentation ||
    (!presentation.os &&
      !presentation.arch &&
      !presentation.capabilities.length)
  )
    return undefined;
  return {
    version: 1,
    shell: presentation.capabilities.includes("shell"),
    files: presentation.capabilities.includes("files"),
    browser: presentation.capabilities.includes("browser"),
    computer: presentation.capabilities.includes("computer"),
    os: presentation.os ?? "Not reported",
    arch: presentation.arch ?? "Not reported",
    roots: presentation.roots,
    computer_mode: presentation.computer_mode,
    cua_version: presentation.cua_version,
    computer_tools: [],
    computer_ready: presentation.desktop_ready,
    browser_isolated: presentation.browser_isolated,
    commands_isolated: presentation.commands_isolated,
    saved_login_ready: false,
  };
}

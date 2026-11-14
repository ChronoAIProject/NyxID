import type { CrashReport } from "../../../../shared/crash-report";
/** Reduce browser metadata locally; the user-agent string is never retained. */
export function coarseRuntime(
  agent: string,
): Pick<CrashReport, "os_version" | "runtime"> {
  const browser = ["Edg", "Firefox", "Chrome", "Version"]
    .map((name) => new RegExp(`(${name})/(\\d+)`).exec(agent))
    .find(Boolean);
  const names = {
    Edg: "edge",
    Firefox: "firefox",
    Chrome: "chrome",
    Version: "safari",
  } as const;
  const os =
    /(?:OS |Mac OS X |Android |Windows NT )(\d{1,3})(?:[._](\d{1,3}))?/.exec(
      agent,
    );
  return {
    os_version: os ? `${os[1]}${os[2] ? `.${os[2]}` : ""}` : "0",
    runtime: {
      name: browser ? names[browser[1] as keyof typeof names] : "other",
      major: Math.min(Number(browser?.[2] ?? 0), 65535),
    },
  };
}

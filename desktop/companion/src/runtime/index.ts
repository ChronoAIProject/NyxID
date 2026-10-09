import type { BrowserRuntimeOptions } from "./browser-runtime";
import { BrowserCompanionRuntime } from "./browser-runtime";
import type { CompanionRuntime } from "./companion-runtime";
import { isTauriHost, TauriCompanionRuntime } from "./tauri-runtime";

export * from "./browser-runtime";
export * from "./companion-runtime";
export * from "./nyxid";
export * from "./tauri-runtime";

export function createCompanionRuntime(
  browserOptions?: BrowserRuntimeOptions,
): CompanionRuntime {
  return isTauriHost()
    ? new TauriCompanionRuntime()
    : new BrowserCompanionRuntime(browserOptions);
}

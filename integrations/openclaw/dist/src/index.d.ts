import type { OpenClawPluginApi } from "./types.js";
declare global {
    var OPENCLAW_NYXID_CONFIG: Record<string, unknown> | undefined;
}
export default function register(api: OpenClawPluginApi): void;
export * from "./client.js";
export * from "./helpers.js";
export * from "./types.js";

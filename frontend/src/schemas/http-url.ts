/**
 * Stricter check for user-entered URLs. Zod's `.url()` accepts anything the
 * WHATWG URL parser swallows — including TLD-less hosts like `https://www.`,
 * `https://foo`, or `javascript:` schemes — which are practically always
 * typos in this app. Rules here:
 *
 * - scheme must be http/https (matches the backend's url_validation.rs,
 *   which rejects everything else — including ws/wss — on every URL field)
 * - hostname must be `localhost`, an IP literal, or a dotted name ending in
 *   an alphabetic TLD of at least 2 characters (`example.com`, `ha.local`)
 *
 * Self-hosted gateways stay valid (e.g. OpenClaw at http://localhost:18789
 * or http://192.168.1.20:8123).
 */

const ALLOWED_PROTOCOLS = new Set(["http:", "https:"]);
const IPV4_RE = /^\d{1,3}(\.\d{1,3}){3}$/;
const HOSTNAME_WITH_TLD_RE =
  /^([a-z0-9]([a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,}$/i;

export function isValidHttpUrl(value: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    return false;
  }
  if (!ALLOWED_PROTOCOLS.has(parsed.protocol)) return false;
  // IPv6 literals come back bracketed, e.g. "[::1]".
  if (parsed.hostname.startsWith("[")) return true;
  const host = parsed.hostname.replace(/\.$/, "").toLowerCase();
  if (host.length === 0) return false;
  if (host === "localhost") return true;
  if (IPV4_RE.test(host)) return true;
  return HOSTNAME_WITH_TLD_RE.test(host);
}

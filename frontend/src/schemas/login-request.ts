import { API_KEY_SCOPES } from "./api-keys";
import { userCodeSchema } from "./auth-device";

export type RequestedPermissions = {
  permissions: string[];
  services: string[];
  service_permissions: string[];
};
export type LoginRequestHints = RequestedPermissions & {
  user_code?: string;
  login_type: "full" | "agent";
  key_source: "existing" | "new";
  key_name: string;
  expiry_days: "7" | "30" | "90" | "365" | "none";
  platform: "generic" | "codex" | "claude-code" | "openclaw";
  show_details: boolean;
  errors: string[];
  query: string;
};

const lists = { permissions: 1024, services: 4096, service_permissions: 8192 };
const singles = [
  "user_code",
  "login_type",
  "key_source",
  "key_name",
  "expiry_days",
  "platform",
  "show_details",
];
const slugPattern = /^[a-z0-9][a-z0-9_-]{0,127}$/;
const hasControls = (value: string) =>
  Array.from(value).some(
    (c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127,
  );

/** Parse the original query, before router coercion can erase duplicates or bad escapes. */
export function parseLoginRequestHints(
  raw: string,
  flow: "device" | "agent-key" = "device",
): LoginRequestHints {
  const result: LoginRequestHints = {
    login_type: flow === "agent-key" ? "agent" : "full",
    key_source: "existing",
    key_name: "",
    expiry_days: "90",
    platform: "generic",
    show_details: false,
    permissions: [],
    services: [],
    service_permissions: [],
    errors: [],
    query: "",
  };
  if (raw.length > 49152) {
    result.errors.push("The request link is too long.");
    return result;
  }
  const params = new URLSearchParams(raw);
  try {
    for (const pair of raw.replace(/^\?/, "").split("&")) {
      for (const part of pair.split("="))
        decodeURIComponent(part.replace(/\+/g, " "));
    }
  } catch {
    result.errors.push("The request link contains malformed encoding.");
  }
  for (const name of new Set(params.keys())) {
    if (!singles.includes(name) && !Object.hasOwn(lists, name))
      result.errors.push(
        `Unsupported request parameter: ${name.slice(0, 64)}.`,
      );
  }
  const single = (name: string): string | undefined => {
    const values = params.getAll(name);
    if (!values.length) return undefined;
    if (values.length !== 1 || hasControls(values[0] ?? "")) {
      result.errors.push(`Invalid or duplicate ${name}.`);
      return undefined;
    }
    return values[0];
  };
  const code = single("user_code");
  if (code !== undefined) {
    const parsed = userCodeSchema.safeParse(code);
    if (parsed.success) result.user_code = parsed.data;
    else result.errors.push("The code in this link is invalid.");
  }
  const showDetails = single("show_details");
  if (showDetails !== undefined) {
    if (showDetails !== "true" && showDetails !== "false")
      result.errors.push("Invalid show_details.");
    else result.show_details = showDetails === "true";
  }
  for (const [name, choices] of [
    ["login_type", ["full", "agent"]],
    ["key_source", ["existing", "new"]],
    ["expiry_days", ["7", "30", "90", "365", "none"]],
    ["platform", ["generic", "codex", "claude-code", "openclaw"]],
  ] as const) {
    const value = single(name);
    if (value === undefined) continue;
    if (!(choices as readonly string[]).includes(value))
      result.errors.push(`Invalid ${name}.`);
    else Object.assign(result, { [name]: value });
  }
  const name = single("key_name");
  if (name !== undefined) {
    if (!name.trim() || name.length > 64)
      result.errors.push("Key name must contain 1–64 characters.");
    else result.key_name = name.trim();
  }
  if (
    result.login_type === "full" &&
    (params.has("key_source") || params.has("key_name"))
  )
    result.errors.push("Key settings require login_type=agent.");
  if (flow === "agent-key" && result.login_type === "full")
    result.errors.push(
      "This request only supports restricted Agent Key access.",
    );
  for (const [name, limit] of Object.entries(lists) as [
    keyof RequestedPermissions,
    number,
  ][]) {
    if (!params.has(name)) continue;
    const rawValues = params.getAll(name).join(",");
    if (rawValues.length > limit || hasControls(rawValues)) {
      result.errors.push(`Invalid or oversized ${name}.`);
      continue;
    }
    const values = [...new Set(rawValues.split(",").map((v) => v.trim()))];
    if (values.some((v) => !v)) {
      result.errors.push(`Empty ${name} value.`);
      continue;
    }
    result[name] = values;
    if (
      name === "permissions" &&
      values.some((v) => !(API_KEY_SCOPES as readonly string[]).includes(v))
    )
      result.errors.push("Unknown NyxID permission in the request link.");
    if (name === "services" && values.some((v) => !slugPattern.test(v)))
      result.errors.push("Invalid service slug in the request link.");
    if (
      name === "service_permissions" &&
      values.some((v) => {
        const parts = v.split("::");
        return (
          parts.length > 2 ||
          (parts.length === 2 &&
            (!slugPattern.test(parts[0] ?? "") || !parts[1])) ||
          v.length > 2048
        );
      })
    )
      result.errors.push("Invalid service permission in the request link.");
  }
  if (!result.errors.length) result.query = params.toString();
  return result;
}

export function loginRequestReturnTo(
  flow: "device" | "agent-key",
  hints: LoginRequestHints,
  code: string,
): string {
  const params = new URLSearchParams(hints.query);
  params.set("user_code", userCodeSchema.parse(code));
  return `/login/${flow}?${params.toString()}`;
}

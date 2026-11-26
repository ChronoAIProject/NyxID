/** Closed diagnostics schema shared by the two first-party JS clients. */
export const exceptionClasses = [
  "Error",
  "TypeError",
  "ReferenceError",
  "RangeError",
  "SyntaxError",
  "EvalError",
  "URIError",
  "AggregateError",
  "ChunkLoadError",
  "NonError",
  "Other",
  "Unknown",
] as const;
export type ExceptionClass = (typeof exceptionClasses)[number];
export const crashKinds = [
  "native_crash",
  "js_fatal",
  "js_error",
  "unhandled_rejection",
  "render_error",
  "recoverable_error",
  "resource_error",
] as const;
export type CrashKind = (typeof crashKinds)[number];
export type JsFrame = { asset: string; line: number; column: number };
export interface CrashReport {
  schema_version: 1;
  event_id: string;
  captured_at: string;
  platform: "web" | "ios" | "android";
  kind: CrashKind;
  exception_class: ExceptionClass;
  native_signal?: string | null;
  app_version: string;
  app_build: string;
  artifact_id?: string | null;
  os_version: string;
  runtime?: {
    name: "chrome" | "safari" | "firefox" | "edge" | "hermes" | "other";
    major: number;
  } | null;
  js_frames: JsFrame[];
  native_images: { uuid: string; name: string; size: number }[];
  native_frames: { image: number | null; offset: number }[];
}
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const eventId =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const object = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
const keys = (v: Record<string, unknown>, allowed: string[]) =>
  Object.keys(v).every((k) => allowed.includes(k));
const integer = (v: unknown, max: number): v is number =>
  typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max;
const text = (v: unknown, pattern: RegExp): v is string =>
  typeof v === "string" && pattern.test(v);
/** Reject unknown keys at every level. Never return or spread untrusted input. */
export function validateCrashRecord(
  input: unknown,
  now = Date.now(),
): CrashReport | null {
  if (
    !object(input) ||
    !keys(input, [
      "schema_version",
      "event_id",
      "captured_at",
      "platform",
      "kind",
      "exception_class",
      "native_signal",
      "app_version",
      "app_build",
      "artifact_id",
      "os_version",
      "runtime",
      "js_frames",
      "native_images",
      "native_frames",
    ])
  )
    return null;
  const v = input;
  if (
    v.schema_version !== 1 ||
    !text(v.event_id, eventId) ||
    typeof v.captured_at !== "string" ||
    v.captured_at.length > 40 ||
    !/^\d{4}-\d{2}-\d{2}T/.test(v.captured_at)
  )
    return null;
  const timestamp = Date.parse(v.captured_at);
  if (
    !Number.isFinite(timestamp) ||
    timestamp < now - 86400000 ||
    timestamp > now + 300000
  )
    return null;
  if (
    !["web", "ios", "android"].includes(String(v.platform)) ||
    !crashKinds.includes(v.kind as CrashKind) ||
    !exceptionClasses.includes(v.exception_class as ExceptionClass)
  )
    return null;
  if (
    !text(v.app_version, /^[0-9A-Za-z.+-]{1,32}$/) ||
    !text(v.app_build, /^[0-9A-Za-z.+-]{1,64}$/) ||
    !text(v.os_version, /^\d{1,3}(\.\d{1,3})?$/)
  )
    return null;
  if (
    (v.artifact_id != null && !text(v.artifact_id, uuid)) ||
    (v.platform === "web" && v.artifact_id == null)
  )
    return null;
  const native = v.kind === "native_crash";
  if (
    native
      ? v.platform !== "ios" ||
        ![
          "SIGSEGV",
          "SIGBUS",
          "SIGABRT",
          "SIGILL",
          "SIGFPE",
          "SIGTRAP",
          "SIGSYS",
          "Other",
        ].includes(String(v.native_signal))
      : v.native_signal != null
  )
    return null;
  const js_frames: JsFrame[] = [],
    native_images: CrashReport["native_images"] = [],
    native_frames: CrashReport["native_frames"] = [];
  if (
    !Array.isArray(v.js_frames) ||
    v.js_frames.length > 64 ||
    !Array.isArray(v.native_images) ||
    v.native_images.length > 16 ||
    !Array.isArray(v.native_frames) ||
    v.native_frames.length > 64
  )
    return null;
  if (
    native
      ? v.js_frames.length > 0 || v.runtime != null
      : v.native_images.length > 0 || v.native_frames.length > 0
  )
    return null;
  for (const f of v.js_frames) {
    if (
      !object(f) ||
      !keys(f, ["asset", "line", "column"]) ||
      !integer(f.line, 1e7) ||
      !integer(f.column, 1e7)
    )
      return null;
    if (
      v.platform === "web"
        ? !text(
            f.asset,
            /^(assets\/[A-Za-z0-9_.-]{1,96}\.js|<external>|<inline>)$/,
          )
        : ![
            "main.jsbundle",
            "index.android.bundle",
            "<native>",
            "<external>",
          ].includes(String(f.asset))
    )
      return null;
    js_frames.push({
      asset: f.asset as string,
      line: f.line,
      column: f.column,
    });
  }
  for (const i of v.native_images) {
    if (
      !object(i) ||
      !keys(i, ["uuid", "name", "size"]) ||
      !text(i.uuid, uuid) ||
      !text(i.name, /^[A-Za-z0-9_.+-]{1,64}$/) ||
      !integer(i.size, Number.MAX_SAFE_INTEGER)
    )
      return null;
    native_images.push({ uuid: i.uuid, name: i.name, size: i.size });
  }
  for (const f of v.native_frames) {
    if (
      !object(f) ||
      !keys(f, ["image", "offset"]) ||
      !integer(f.offset, Number.MAX_SAFE_INTEGER) ||
      (f.image !== null && !integer(f.image, native_images.length - 1))
    )
      return null;
    if (typeof f.image === "number" && f.offset >= native_images[f.image]!.size)
      return null;
    native_frames.push({ image: f.image as number | null, offset: f.offset });
  }
  let runtime: CrashReport["runtime"];
  if (v.runtime != null) {
    const r = v.runtime;
    if (
      !object(r) ||
      !keys(r, ["name", "major"]) ||
      !integer(r.major, 65535) ||
      !["chrome", "safari", "firefox", "edge", "hermes", "other"].includes(
        String(r.name),
      ) ||
      (v.platform !== "web" && r.name !== "hermes") ||
      (v.platform === "web" && r.name === "hermes")
    )
      return null;
    runtime = {
      name: r.name as NonNullable<CrashReport["runtime"]>["name"],
      major: r.major,
    };
  }
  return {
    schema_version: 1,
    event_id: v.event_id,
    captured_at: v.captured_at,
    platform: v.platform as CrashReport["platform"],
    kind: v.kind as CrashKind,
    exception_class: v.exception_class as ExceptionClass,
    app_version: v.app_version,
    app_build: v.app_build,
    os_version: v.os_version,
    ...(v.artifact_id != null ? { artifact_id: v.artifact_id as string } : {}),
    ...(native ? { native_signal: v.native_signal as string } : {}),
    ...(runtime ? { runtime } : {}),
    js_frames,
    native_images,
    native_frames,
  };
}
export function exceptionClass(error: unknown): ExceptionClass {
  if (!(error instanceof Error)) return "NonError";
  return exceptionClasses.includes(error.name as ExceptionClass)
    ? (error.name as ExceptionClass)
    : "Other";
}

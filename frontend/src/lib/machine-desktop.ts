const HEADER = 30;
const MAGIC = [78, 89, 88, 77];
export interface DesktopFrame {
  sequence: bigint;
  image: Blob;
  rectangle?: {
    width: number;
    height: number;
    x: number;
    y: number;
    patchWidth: number;
    patchHeight: number;
    base: bigint;
  };
}
export function desktopFrame(
  buffer: ArrayBuffer,
  session: string,
): DesktopFrame | undefined {
  const bytes = new Uint8Array(buffer);
  if (
    bytes.length < HEADER ||
    bytes.length > HEADER + 5 * 1024 * 1024 ||
    MAGIC.some((byte, i) => bytes[i] !== byte) ||
    bytes[4] !== 1
  )
    return;
  const id = Array.from(bytes.slice(6, 22), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
  if (id !== session.replaceAll("-", "")) return;
  const view = new DataView(buffer);
  const sequence = view.getBigUint64(22);
  if (
    bytes.length >= HEADER + 24 &&
    bytes
      .slice(HEADER, HEADER + 4)
      .every((value, i) => value === [78, 89, 88, 68][i])
  ) {
    const rectangle = {
      width: view.getUint16(HEADER + 4),
      height: view.getUint16(HEADER + 6),
      x: view.getUint16(HEADER + 8),
      y: view.getUint16(HEADER + 10),
      patchWidth: view.getUint16(HEADER + 12),
      patchHeight: view.getUint16(HEADER + 14),
      base: view.getBigUint64(HEADER + 16),
    };
    if (
      !rectangle.width ||
      !rectangle.height ||
      rectangle.width > 1920 ||
      rectangle.height > 1200 ||
      !rectangle.patchWidth ||
      !rectangle.patchHeight ||
      rectangle.x + rectangle.patchWidth > rectangle.width ||
      rectangle.y + rectangle.patchHeight > rectangle.height
    )
      return;
    if (
      rectangle.base === 0n &&
      (rectangle.x !== 0 ||
        rectangle.y !== 0 ||
        rectangle.patchWidth !== rectangle.width ||
        rectangle.patchHeight !== rectangle.height)
    )
      return;
    return {
      sequence,
      rectangle,
      image: new Blob([buffer.slice(HEADER + 24)], { type: "image/jpeg" }),
    };
  }
  return {
    sequence,
    image: new Blob([buffer.slice(HEADER)], { type: "image/jpeg" }),
  };
}
export function desktopInput(
  session: string,
  sequence: bigint,
  tool: string,
  args: Record<string, unknown>,
): ArrayBuffer {
  const data = new TextEncoder().encode(
    JSON.stringify({ tool, arguments: args }),
  );
  if (
    data.length > 16384 ||
    !/^[\da-f]{8}(-[\da-f]{4}){3}-[\da-f]{12}$/.test(session)
  )
    throw new Error("Invalid desktop input");
  const bytes = new Uint8Array(HEADER + data.length);
  bytes.set(MAGIC);
  bytes[4] = 2;
  const uuid = session.replaceAll("-", "");
  for (let i = 0; i < 16; i++)
    bytes[6 + i] = Number.parseInt(uuid.slice(i * 2, i * 2 + 2), 16);
  new DataView(bytes.buffer).setBigUint64(22, sequence);
  bytes.set(data, HEADER);
  return bytes.buffer;
}
export function desktopUrl(
  node: string,
  conversation?: string,
  backend?: string,
): string {
  const url = new URL(backend || window.location.origin);
  url.protocol =
    url.protocol === "https:" || url.protocol === "wss:" ? "wss:" : "ws:";
  url.pathname = `/api/v1/assistant/nyxagent/machines/${encodeURIComponent(node)}/desktop`;
  url.search = conversation
    ? new URLSearchParams({ conversation_id: conversation }).toString()
    : "";
  url.hash = "";
  return url.toString();
}

/** Convert a pointer through object-contain letterboxing into capture pixels. */
export function desktopPoint(
  rect: { left: number; top: number; width: number; height: number },
  width: number,
  height: number,
  clientX: number,
  clientY: number,
): { x: number; y: number } | undefined {
  if (!width || !height || !rect.width || !rect.height) return;
  const scale = Math.min(rect.width / width, rect.height / height);
  const x = (clientX - rect.left - (rect.width - width * scale) / 2) / scale;
  const y = (clientY - rect.top - (rect.height - height * scale) / 2) / scale;
  if (x < 0 || y < 0 || x >= width || y >= height) return;
  return { x, y };
}

export function desktopKey(key: string): string {
  const names: Record<string, string> = {
    ArrowLeft: "LEFT",
    ArrowRight: "RIGHT",
    ArrowUp: "UP",
    ArrowDown: "DOWN",
    Enter: "ENTER",
    Escape: "ESC",
    Backspace: "BACKSPACE",
    Delete: "DELETE",
    Tab: "TAB",
    Home: "HOME",
    End: "END",
    PageUp: "PAGEUP",
    PageDown: "PAGEDOWN",
    " ": "SPACE",
  };
  return names[key] ?? key.toUpperCase();
}

export interface DesktopActivity {
  tool: string;
  cursor?: { x: number; y: number };
}
export function desktopActivity(
  buffer: ArrayBuffer,
  session: string,
): DesktopActivity | undefined {
  const bytes = new Uint8Array(buffer);
  if (
    bytes.length < HEADER ||
    bytes.length > HEADER + 1024 ||
    MAGIC.some((byte, i) => bytes[i] !== byte) ||
    bytes[4] !== 11
  )
    return;
  const id = Array.from(bytes.slice(6, 22), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
  if (id !== session.replaceAll("-", "")) return;
  try {
    const value = JSON.parse(
      new TextDecoder().decode(bytes.slice(HEADER)),
    ) as DesktopActivity;
    if (!/^[a-z_]{1,40}$/.test(value.tool)) return;
    const cursor = value.cursor;
    if (
      cursor &&
      (!Number.isFinite(cursor.x) ||
        !Number.isFinite(cursor.y) ||
        cursor.x < 0 ||
        cursor.x > 1 ||
        cursor.y < 0 ||
        cursor.y > 1)
    )
      return;
    return value;
  } catch {
    return;
  }
}

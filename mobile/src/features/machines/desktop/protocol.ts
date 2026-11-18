/** Pure NYXM/NYXD byte contract, shared by native admission and canvas tests.
 * No Blob, filesystem, React Native or browser APIs. All sequences stay u64. */
export const HEADER_BYTES = 30;
export const MAX_JPEG_BYTES = 5 * 1024 * 1024;
export const MAX_WIDTH = 1920;
export const MAX_HEIGHT = 1200;
export type Rectangle = { width: number; height: number; x: number; y: number; patchWidth: number; patchHeight: number; base: bigint };
export type DesktopFrame = { sequence: bigint; end: boolean; jpeg: Uint8Array; width: number; height: number; rectangle?: Rectangle };
export type Envelope = { kind: 1 | 2 | 11; end: boolean; sequence: bigint; payload: Uint8Array };
const uuid = /^[\da-f]{8}(-[\da-f]{4}){3}-[\da-f]{12}$/i;
export function envelope(buffer: ArrayBuffer, session: string): Envelope | undefined {
  const bytes = new Uint8Array(buffer);
  if (bytes.length < HEADER_BYTES || bytes.length > HEADER_BYTES + MAX_JPEG_BYTES ||
      bytes[0] !== 78 || bytes[1] !== 89 || bytes[2] !== 88 || bytes[3] !== 77 ||
      ![1, 2, 11].includes(bytes[4]!) || bytes[5]! > 1 || !uuid.test(session)) return;
  const id = Array.from(bytes.subarray(6, 22), b => b.toString(16).padStart(2, '0')).join('');
  if (id !== session.replaceAll('-', '').toLowerCase()) return;
  return { kind: bytes[4] as 1 | 2 | 11, end: bytes[5] === 1,
    sequence: new DataView(buffer).getBigUint64(22, false), payload: bytes.subarray(30) };
}
/** Inspect dimensions before asking a decoder to allocate a bitmap. The actual
 * decoder must independently check its natural dimensions before compositing. */
export function jpegSize(bytes: Uint8Array): { width: number; height: number } | undefined {
  if (bytes.length < 10 || bytes[0] !== 255 || bytes[1] !== 216 ||
      bytes[bytes.length - 2] !== 255 || bytes[bytes.length - 1] !== 217) return;
  let at = 2;
  while (at + 4 < bytes.length) {
    if (bytes[at++] !== 255) return;
    while (bytes[at] === 255) at++;
    const marker = bytes[at++];
    if (marker === undefined || marker === 218 || marker === 217) return;
    if (marker === 1 || (marker >= 208 && marker <= 215)) continue;
    const length = (bytes[at]! << 8) | bytes[at + 1]!;
    if (length < 2 || at + length > bytes.length) return;
    if ([192,193,194,195,197,198,199,201,202,203,205,206,207].includes(marker)) {
      if (length < 8) return;
      const height = (bytes[at + 3]! << 8) | bytes[at + 4]!;
      const width = (bytes[at + 5]! << 8) | bytes[at + 6]!;
      if (!width || !height || width > MAX_WIDTH || height > MAX_HEIGHT) return;
      return { width, height };
    }
    at += length;
  }
}
export function desktopFrame(buffer: ArrayBuffer, session: string): DesktopFrame | undefined {
  const frame = envelope(buffer, session);
  if (!frame || frame.kind !== 1 || frame.sequence === 0n) return;
  const bytes = frame.payload;
  let rectangle: Rectangle | undefined;
  let jpeg = bytes;
  if (bytes[0] === 78 && bytes[1] === 89 && bytes[2] === 88 && bytes[3] === 68) {
    if (bytes.length < 24) return;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    rectangle = { width: view.getUint16(4), height: view.getUint16(6), x: view.getUint16(8), y: view.getUint16(10),
      patchWidth: view.getUint16(12), patchHeight: view.getUint16(14), base: view.getBigUint64(16) };
    const r = rectangle;
    if (!r.width || !r.height || r.width > MAX_WIDTH || r.height > MAX_HEIGHT || !r.patchWidth || !r.patchHeight ||
        r.x + r.patchWidth > r.width || r.y + r.patchHeight > r.height ||
        (r.base === 0n && (r.x !== 0 || r.y !== 0 || r.patchWidth !== r.width || r.patchHeight !== r.height))) return;
    jpeg = bytes.subarray(24);
  }
  const size = jpegSize(jpeg);
  if (!size || (rectangle && (size.width !== rectangle.patchWidth || size.height !== rectangle.patchHeight))) return;
  return { sequence: frame.sequence, end: frame.end, jpeg, ...size, rectangle };
}
export function desktopActivity(buffer: ArrayBuffer, session: string): { tool: string } | undefined {
  const frame = envelope(buffer, session);
  if (!frame || frame.kind !== 11 || frame.payload.length > 1024) return;
  try {
    const value: unknown = JSON.parse(Array.from(frame.payload, b => String.fromCharCode(b)).join(''));
    if (typeof value !== 'object' || value === null) return;
    const { tool, cursor } = value as { tool?: unknown; cursor?: {x?: unknown; y?: unknown} };
    if (typeof tool !== 'string' || !/^[a-z_]{1,40}$/.test(tool)) return;
    if (cursor && (typeof cursor.x !== 'number' || typeof cursor.y !== 'number' || !Number.isFinite(cursor.x) ||
      !Number.isFinite(cursor.y) || cursor.x < 0 || cursor.x > 1 || cursor.y < 0 || cursor.y > 1)) return;
    return { tool };
  } catch { return; }
}
export function base64(bytes: Uint8Array): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const chunks: string[] = [];
  for (let start = 0; start < bytes.length; start += 12288) {
    let chunk = '';
    for (let i = start; i < Math.min(bytes.length, start + 12288); i += 3) {
      const a = bytes[i]!, b = bytes[i + 1], c = bytes[i + 2];
      chunk += alphabet[a >> 2]! + alphabet[((a & 3) << 4) | ((b ?? 0) >> 4)]! +
        (b === undefined ? '=' : alphabet[((b & 15) << 2) | ((c ?? 0) >> 6)]!) +
        (c === undefined ? '=' : alphabet[c & 63]!);
    }
    chunks.push(chunk);
  }
  return chunks.join('');
}

export type DesktopInput =
  | { tool: 'click'; arguments: { x: number; y: number; button: 'left' | 'right'; count: number } }
  | { tool: 'drag'; arguments: { from_x: number; from_y: number; to_x: number; to_y: number; button: 'left'; duration_ms: number } }
  | { tool: 'scroll'; arguments: { x: number; y: number; direction: 'up' | 'down' | 'left' | 'right'; amount: number } }
  | { tool: 'type_text'; arguments: { text: string } }
  | { tool: 'press_key'; arguments: { key: 'ENTER' | 'TAB' | 'BACKSPACE' | 'ESC' | 'LEFT' | 'RIGHT' | 'UP' | 'DOWN' } };
const coordinate = (value: unknown, maximum: number): value is number => typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= maximum;
/** Validate renderer messages again at the native socket boundary. */
export function desktopInputValue(value: unknown): DesktopInput | undefined {
  if (!value || typeof value !== 'object') return;
  const { tool, arguments: args } = value as { tool?: unknown; arguments?: Record<string, unknown> };
  if (!args || typeof args !== 'object') return;
  switch (tool) {
    case 'click': if (coordinate(args.x, MAX_WIDTH) && coordinate(args.y, MAX_HEIGHT) && ['left', 'right'].includes(String(args.button)) && args.count === 1)
      return { tool, arguments: { x: args.x, y: args.y, button: args.button as 'left' | 'right', count: 1 } }; break;
    case 'drag': if (coordinate(args.from_x, MAX_WIDTH) && coordinate(args.to_x, MAX_WIDTH) && coordinate(args.from_y, MAX_HEIGHT) && coordinate(args.to_y, MAX_HEIGHT) && args.button === 'left' && typeof args.duration_ms === 'number' && Number.isFinite(args.duration_ms))
      return { tool, arguments: { from_x: args.from_x as number, from_y: args.from_y as number, to_x: args.to_x as number, to_y: args.to_y as number, button: 'left', duration_ms: Math.min(10000, Math.max(0, Math.round(args.duration_ms))) } }; break;
    case 'scroll': if (coordinate(args.x, MAX_WIDTH) && coordinate(args.y, MAX_HEIGHT) && ['up', 'down', 'left', 'right'].includes(String(args.direction)) && args.amount === 3)
      return { tool, arguments: { x: args.x, y: args.y, direction: args.direction as 'up' | 'down' | 'left' | 'right', amount: 3 } }; break;
    case 'type_text': if (typeof args.text === 'string' && args.text.length > 0 && args.text.length <= 8192)
      return { tool, arguments: { text: args.text } }; break;
    case 'press_key': if (['ENTER', 'TAB', 'BACKSPACE', 'ESC', 'LEFT', 'RIGHT', 'UP', 'DOWN'].includes(String(args.key)))
      return { tool, arguments: { key: args.key as Extract<DesktopInput, { tool: 'press_key' }>['arguments']['key'] } }; break;
  }
}
/** Same NYXM kind=2 UUID/u64 and JSON {tool,arguments} as the web client. */
export function desktopInput(session: string, sequence: bigint, input: DesktopInput): ArrayBuffer {
  const command = desktopInputValue(input);
  if (!uuid.test(session) || sequence <= 0n || sequence > 0xffffffffffffffffn || !command) throw new Error('Invalid desktop input');
  // RN/Hermes does not guarantee TextEncoder. Encode UTF-8 explicitly.
  const payload: number[] = [];
  for (const character of JSON.stringify(command)) {
    const code = character.codePointAt(0)!;
    if (code < 0x80) payload.push(code);
    else if (code < 0x800) payload.push(0xc0 | code >> 6, 0x80 | code & 63);
    else if (code < 0x10000) payload.push(0xe0 | code >> 12, 0x80 | code >> 6 & 63, 0x80 | code & 63);
    else payload.push(0xf0 | code >> 18, 0x80 | code >> 12 & 63, 0x80 | code >> 6 & 63, 0x80 | code & 63);
  }
  if (payload.length > 16384) throw new Error('Desktop input is too long');
  const bytes = new Uint8Array(HEADER_BYTES + payload.length);
  bytes.set([78, 89, 88, 77, 2, 0]);
  const id = session.replaceAll('-', '');
  for (let i = 0; i < 16; i++) bytes[6 + i] = Number.parseInt(id.slice(i * 2, i * 2 + 2), 16);
  new DataView(bytes.buffer).setBigUint64(22, sequence, false);
  bytes.set(payload, HEADER_BYTES);
  return bytes.buffer;
}

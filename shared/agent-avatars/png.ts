/** Portable deterministic RGBA PNG encoding for avatars and sampled backgrounds. */
const u32 = (n: number) =>
  new Uint8Array([n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255]);
function concat(parts: readonly Uint8Array[]) {
  const out = new Uint8Array(parts.reduce((s, p) => s + p.length, 0));
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}
const CRC_TABLE = Uint32Array.from({ length: 256 }, (_, value) => {
  let c = value;
  for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (c & 1 ? 0xedb88320 : 0);
  return c >>> 0;
});
function* crcSteps(bytes: Uint8Array): Generator<void, number> {
  let c = 0xffffffff;
  for (let offset = 0; offset < bytes.length; offset += 8192) {
    const end = Math.min(bytes.length, offset + 8192);
    for (let i = offset; i < end; i++)
      c = (c >>> 8) ^ CRC_TABLE[(c ^ bytes[i]!) & 255]!;
    yield;
  }
  return (c ^ 0xffffffff) >>> 0;
}

function* chunkSteps(
  type: string,
  data: Uint8Array,
): Generator<void, Uint8Array> {
  const content = concat([
    new Uint8Array([...type].map((c) => c.charCodeAt(0))),
    data,
  ]);
  const checksum = yield* crcSteps(content);
  return concat([u32(data.length), content, u32(checksum)]);
}

export function encodePng(
  width: number,
  height: number,
  pixels: Uint8Array,
): Uint8Array {
  const steps = encodePngSteps(width, height, pixels);
  let next = steps.next();
  while (!next.done) next = steps.next();
  return next.value;
}

function* encodePngSteps(
  width: number,
  height: number,
  pixels: Uint8Array,
): Generator<void, Uint8Array> {
  if (pixels.length !== width * height * 4)
    throw new Error("Invalid RGBA dimensions");
  const raw = new Uint8Array(height * (width * 4 + 1));
  for (let row = 0; row < height; row++) {
    raw.set(
      pixels.subarray(row * width * 4, (row + 1) * width * 4),
      row * (width * 4 + 1) + 1,
    );
    if (row % 16 === 0) yield;
  }
  const blocks: Uint8Array[] = [new Uint8Array([0x78, 0x01])];
  for (let offset = 0; offset < raw.length; offset += 65535) {
    const len = Math.min(65535, raw.length - offset);
    blocks.push(
      new Uint8Array([
        offset + len === raw.length ? 1 : 0,
        len & 255,
        len >>> 8,
        ~len & 255,
        (~len >>> 8) & 255,
      ]),
      raw.subarray(offset, offset + len),
    );
    yield;
  }
  let a = 1,
    b = 0;
  // The standard Adler-32 block bound keeps sums exact with two modulo
  // operations per block, avoiding expensive per-byte remainder on Hermes.
  for (let offset = 0; offset < raw.length; offset += 5552) {
    const end = Math.min(raw.length, offset + 5552);
    for (let i = offset; i < end; i++) {
      a += raw[i]!;
      b += a;
    }
    a %= 65521;
    b %= 65521;
    yield;
  }
  blocks.push(u32((b << 16) | a));
  const header = yield* chunkSteps(
    "IHDR",
    concat([u32(width), u32(height), new Uint8Array([8, 6, 0, 0, 0])]),
  );
  const data = yield* chunkSteps("IDAT", concat(blocks));
  const end = yield* chunkSteps("IEND", new Uint8Array());
  return concat([
    new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    header,
    data,
    end,
  ]);
}

/** Same encoder and bytes, with bounded CRC/Adler/copy work between yields. */
export async function encodePngAsync(
  width: number,
  height: number,
  pixels: Uint8Array,
  options: { check?: () => void; onYield?: () => void } = {},
): Promise<Uint8Array> {
  const steps = encodePngSteps(width, height, pixels);
  let next = steps.next(),
    started = performance.now();
  while (!next.done) {
    options.check?.();
    if (performance.now() - started >= 8) {
      options.onYield?.();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      options.check?.();
      started = performance.now();
    }
    next = steps.next();
  }
  options.check?.();
  return next.value;
}
export function pngDataUri(bytes: Uint8Array): string {
  const alphabet =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    parts: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 3072) {
    let block = "";
    const end = Math.min(bytes.length, offset + 3072);
    for (let i = offset; i < end; i += 3) {
      const a = bytes[i]!,
        b = bytes[i + 1] ?? 0,
        c = bytes[i + 2] ?? 0,
        n = (a << 16) | (b << 8) | c;
      block +=
        alphabet[(n >>> 18) & 63]! +
        alphabet[(n >>> 12) & 63]! +
        (i + 1 < bytes.length ? alphabet[(n >>> 6) & 63] : "=") +
        (i + 2 < bytes.length ? alphabet[n & 63] : "=");
    }
    parts.push(block);
  }
  return "data:image/png;base64," + parts.join("");
}

export interface PixelPlane {
  readonly width: number;
  readonly height: number;
  readonly rgba: Uint8Array;
}
const images = new WeakMap<PixelPlane, string>();
export function planeDataUri(plane: PixelPlane): string {
  let uri = images.get(plane);
  if (!uri) {
    uri = pngDataUri(encodePng(plane.width, plane.height, plane.rgba));
    images.set(plane, uri);
  }
  return uri;
}

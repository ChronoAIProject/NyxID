const STRUCTURAL_CHUNK_BYTES = 4096;

function isJsonWhitespace(byte: number) {
  return byte === 0x20 || byte === 0x09 || byte === 0x0a || byte === 0x0d;
}

async function outerByte(blob: Blob, fromEnd: boolean): Promise<number | undefined> {
  for (let offset = 0; offset < blob.size; offset += STRUCTURAL_CHUNK_BYTES) {
    const start = fromEnd ? Math.max(0, blob.size - offset - STRUCTURAL_CHUNK_BYTES) : offset;
    const end = fromEnd ? blob.size - offset : Math.min(blob.size, offset + STRUCTURAL_CHUNK_BYTES);
    const bytes = new Uint8Array(await blob.slice(start, end).arrayBuffer());
    for (let i = 0; i < bytes.length; i++) {
      const byte = bytes[fromEnd ? bytes.length - 1 - i : i];
      if (byte !== undefined && !isJsonWhitespace(byte)) return byte;
    }
  }
  return undefined;
}

/** Validate completeness without retaining a full body string when length proves it. */
export async function validateAccountExport(blob: Blob, headers: Pick<Headers, "get">): Promise<void> {
  const encoding = headers.get("Content-Encoding");
  // Fetch exposes decoded bytes, not the compressed wire representation.
  const length = encoding !== null && encoding.trim().toLowerCase() !== "identity"
    ? null : headers.get("Content-Length");
  if (length !== null) {
    if (!/^\d+$/.test(length) || blob.size !== Number(length)) {
      throw new Error("Incomplete account export");
    }
    // Only bounded slices are read; save the original blob directly afterwards.
    if (await outerByte(blob, false) !== 0x7b || await outerByte(blob, true) !== 0x7d) {
      throw new Error("Invalid account export");
    }
    return;
  }
  const data: unknown = JSON.parse(await blob.text());
  if (!data || typeof data !== "object" || !("format" in data) || data.format !== "nyxid.account-export") {
    throw new Error("Invalid account export");
  }
}

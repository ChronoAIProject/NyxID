/** Bound native downloads while consuming the stream, before saving or decoding. */
export async function readBoundedBytes(
  body: ReadableStream<Uint8Array> | null,
  maximum: number,
): Promise<Uint8Array> {
  if (!body) throw new Error("The attachment download is unavailable.");
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > maximum) {
        await reader.cancel().catch(() => undefined);
        throw new Error("The attachment exceeds the download limit.");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

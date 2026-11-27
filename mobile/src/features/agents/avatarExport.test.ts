import test from "node:test";
import assert from "node:assert/strict";
import { inflateSync } from "node:zlib";
import {
  pngAvatar,
  pngAvatarAsync,
  rasterAvatar,
} from "../../../../shared/agent-avatars/export";
import { defaultAvatar } from "../../../../shared/agent-avatars/model";
import { drawAvatar } from "../../../../shared/agent-avatars/draw";
function validate(bytes: Uint8Array) {
  const b = Buffer.from(bytes);
  assert.equal(b.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  const types: string[] = [];
  let compressed: Buffer | undefined;
  for (let i = 8; i < b.length; ) {
    const len = b.readUInt32BE(i),
      type = b.subarray(i + 4, i + 8).toString(),
      data = b.subarray(i + 8, i + 8 + len);
    types.push(type);
    let crc = 0xffffffff;
    for (const byte of b.subarray(i + 4, i + 8 + len)) {
      crc ^= byte;
      for (let bit = 0; bit < 8; bit++)
        crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
    assert.equal((crc ^ 0xffffffff) >>> 0, b.readUInt32BE(i + 8 + len));
    if (type === "IHDR") {
      assert.equal(data.readUInt32BE(0), 512);
      assert.equal(data.readUInt32BE(4), 512);
      assert.equal(data[8], 8);
      assert.equal(data[9], 6);
    }
    if (type === "IDAT") compressed = data;
    i += 12 + len;
  }
  assert.deepEqual(types, ["IHDR", "IDAT", "IEND"]);
  const raw = inflateSync(compressed!);
  assert.equal(raw.length, 512 * 2049);
  return raw;
}
test("yielded native export matches synchronous bytes with alpha, apertures and textured surfaces", async () => {
  for (const surface of [
    "flat",
    "ascii-inverted",
    "dither-halftone",
    "dither-phosphor",
  ]) {
    const base = defaultAvatar("nyx", "nyxbot");
    const config = { ...base, choices: { ...base.choices, surface } };
    let tick = false;
    setTimeout(() => {
      tick = true;
    }, 0);
    const asyncBytes = await pngAvatarAsync(config, true, "dark", "nyxbot");
    assert.ok(
      tick,
      "navigation/event loop gets control before export completes",
    );
    const sync = pngAvatar(config, true, "dark", "nyxbot");
    assert.deepEqual(asyncBytes, sync);
    validate(sync);
  }
  const base = defaultAvatar("nyx", "nyxbot");
  const config = {
    ...base,
    choices: { ...base.choices, treatment: "neon-line" },
  };
  const raw = validate(pngAvatar(config, true));
  assert.ok(raw.some((byte, i) => (i % 2049) % 4 === 0 && byte > 0));
  const pixels = rasterAvatar(
    drawAvatar(defaultAvatar("nyx", "nyxbot"), { resting: true, size: 512 }),
  );
  assert.equal(pixels[(300 * 512 + 256) * 4 + 3], 255);
});
test("an authority change cancels yielded export before completion", async () => {
  let checks = 0;
  await assert.rejects(
    pngAvatarAsync(defaultAvatar("nyx", "nyxbot"), false, "dark", "nyxbot", {
      check: () => {
        if (++checks > 3) throw Error("actor changed");
      },
    }),
    /actor changed/,
  );
});

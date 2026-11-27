import { paintAvatar } from "../src/components/assistant/avatar/canvas";
import {
  defaultAvatar,
  AVATAR_CATALOGUE,
} from "../../shared/agent-avatars/model";
import { drawAvatar } from "../../shared/agent-avatars/draw";
import { rasterAvatar, pngAvatar } from "../../shared/agent-avatars/export";
import { pngDataUri } from "../../shared/agent-avatars/png";

const base = defaultAvatar("nyx", "nyxbot");
export async function comparePixels() {
  const results = [];
  for (const surface of Object.keys(AVATAR_CATALOGUE.fields.surface)) {
    const scene = drawAvatar(
      { ...base, choices: { ...base.choices, surface } },
      { resting: true, size: 512 },
    );
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 512;
    const context = canvas.getContext("2d")!;
    paintAvatar(context, scene);
    const actual = context.getImageData(0, 0, 512, 512).data,
      expected = rasterAvatar(scene);
    let sum = 0,
      interior = 0,
      edges = 0;
    for (let y = 2; y < 510; y++)
      for (let x = 2; x < 510; x++) {
        const at = (y * 512 + x) * 4;
        let edge = false;
        for (const delta of [-8, -4, 4, 8, -4096, -2048, 2048, 4096])
          for (let c = 0; c < 4; c++)
            if (
              Math.abs(expected[at + c]! - expected[at + delta + c]!) > 12 ||
              Math.abs(actual[at + c]! - actual[at + delta + c]!) > 12
            )
              edge = true;
        for (let c = 0; c < 4; c++) {
          const error = Math.abs(actual[at + c]! - expected[at + c]!);
          sum += error;
          if (!edge && error > 8) interior++;
        }
        if (edge) edges++;
      }
    const image = new Image();
    image.src = pngDataUri(
      pngAvatar({ ...base, choices: { ...base.choices, surface } }),
    );
    await image.decode();
    results.push({
      surface,
      mean: sum / (508 * 508 * 4),
      interior,
      edges,
      width: image.naturalWidth,
      height: image.naturalHeight,
    });
  }
  return results;
}

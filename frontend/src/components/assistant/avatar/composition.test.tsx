import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { defaultAvatar } from "@/lib/assistant/agent-avatar-model";
import {
  drawAvatar,
  faceMask,
  FACE_GEOMETRY,
} from "../../../../../shared/agent-avatars/draw";
import { rasterAvatar } from "../../../../../shared/agent-avatars/export";
import { AvatarSvg } from "./avatar-svg";
const base = defaultAvatar("nyx", "nyxbot");
it("paints the gradient face above every backdrop with real apertures", () => {
  for (const background of [
    "treatment",
    "dark",
    "solid",
    "gradient",
    "mesh",
    "glass",
    "transparent",
    "ripple",
  ]) {
    const scene = drawAvatar(
      { ...base, choices: { ...base.choices, background } },
      { resting: true, theme: "dark" },
    );
    const pixels = rasterAvatar(scene),
      mask = faceMask("round", "tall", 0.12);
    let bright = 0,
      samples = 0;
    for (let y = 160; y < 360; y += 8)
      for (let x = 160; x < 360; x += 8) {
        if (!mask.sample((x + 0.5) / 512, (y + 0.5) / 512)) continue;
        const at = (y * 512 + x) * 4;
        samples++;
        if (Math.max(pixels[at]!, pixels[at + 1]!, pixels[at + 2]!) > 70)
          bright++;
      }
    if (background === "treatment" || background === "dark")
      expect(bright, background).toBeGreaterThan(samples * 0.9);
    const html = renderToStaticMarkup(<AvatarSvg scene={scene} />);
    if (scene.backgroundPaths.length)
      expect(html.indexOf(scene.backgroundPaths[0]!.d)).toBeLessThan(
        html.lastIndexOf("<image"),
      );
    expect(scene.foregroundPlane).toBeDefined();
    // Removing the foreground must visibly change the actual export pixels.
    const backdrop = rasterAvatar({ ...scene, foregroundPlane: undefined });
    let changed = 0;
    for (let i = 0; i < pixels.length; i += 4)
      if (pixels[i] !== backdrop[i] || pixels[i + 2] !== backdrop[i + 2])
        changed++;
    expect(changed, background).toBeGreaterThan(40000);
  }
});
it("keeps a contrasting crisp voice face on all twelve ripple rings", () => {
  for (const [id, p] of Object.entries(FACE_GEOMETRY.ripples)) {
    const scene = drawAvatar(
      {
        ...base,
        choices: { ...base.choices, background: "transparent", frame: "none" },
      },
      { resting: true, ambient: p.ring, theme: p.light ? "light" : "dark" },
    );
    const pixels = rasterAvatar(scene);
    expect(pixels[(300 * 512 + 256) * 4 + 3], id).toBe(255);
    expect(scene.foregroundPlane, id).toBeDefined();
  }
});

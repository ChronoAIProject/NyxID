/** Run from frontend after capture-nyxbot-media.mjs. Requires Pillow in the
 * project capture venv; the app itself gains no media-encoding dependency. */
import { chromium } from "@playwright/test";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";

const scratch = resolve("../.playwright-mcp/app-media");
const python = resolve("../.playwright-mcp/media-venv/bin/python3");
const output = resolve("public/landing/nyx/app");
const browser = await chromium.launch();
try {
  for (const theme of ["light", "dark"]) for (const mobile of [false, true]) {
    const suffix = `${theme}${mobile ? "-mobile" : ""}`;
    const frames = resolve(scratch, `gif-frames-${suffix}`);
    await rm(frames, { recursive: true, force: true });
    await mkdir(frames, { recursive: true });
    const clip = await readFile(resolve(scratch, "recordings", `task-${suffix}.webm`));
    const page = await browser.newPage();
    await page.setContent('<video muted preload="auto"></video><canvas></canvas>');
    const dimensions = await page.evaluate(async data => {
      const video = document.querySelector("video");
      video.src = `data:video/webm;base64,${data}`;
      await new Promise(resolve => video.addEventListener("loadedmetadata", resolve, { once: true }));
      return { width: video.videoWidth, height: video.videoHeight, duration: video.duration };
    }, clip.toString("base64"));
    // Start after the message has been sent, so the loop contains the task and
    // result rather than an unrelated home empty state or partially typed text.
    const start = 4.5, end = dimensions.duration - 0.2, fps = 8;
    const count = Math.ceil((end - start) * fps);
    for (let index = 0; index < count; index++) {
      const png = await page.evaluate(async seconds => {
        const video = document.querySelector("video"), canvas = document.querySelector("canvas");
        video.currentTime = seconds;
        await new Promise(resolve => video.addEventListener("seeked", resolve, { once: true }));
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        canvas.getContext("2d").drawImage(video, 0, 0);
        return canvas.toDataURL("image/png").split(",")[1];
      }, Math.min(end, start + index / fps));
      await writeFile(resolve(frames, `${String(index).padStart(3, "0")}.png`), Buffer.from(png, "base64"));
    }
    await page.close();
    // A shared palette keeps the paper and type stable. GIF's changed-frame
    // rectangles keep a mostly static app capture small without blurry scaling.
    execFileSync(python, ["-c", `
import sys
from pathlib import Path
from PIL import Image
paths=sorted(Path(sys.argv[1]).glob('*.png'))
images=[Image.open(p).convert('RGB') for p in paths]
w,h=images[0].size
sample=Image.new('RGB',(w,h*3))
for i,index in enumerate([0,len(images)//2,len(images)-1]): sample.paste(images[index],(0,h*i))
palette=sample.quantize(colors=128,method=Image.Quantize.MEDIANCUT)
frames=[image.quantize(palette=palette,dither=Image.Dither.NONE) for image in images]
sequence=[frames[-1]]+frames
sequence[0].save(sys.argv[2],save_all=True,append_images=sequence[1:],duration=[1500]+[125]*len(frames),loop=0,optimize=True,disposal=1)
`, frames, resolve(output, `task-${suffix}.gif`)]);
    const bytes = (await readFile(resolve(output, `task-${suffix}.gif`))).length;
    console.log(JSON.stringify({ suffix, ...dimensions, frames: count, bytes }));
  }
} finally {
  await browser.close();
}

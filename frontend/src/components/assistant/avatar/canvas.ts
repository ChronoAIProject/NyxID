import type { PixelPlane } from "../../../../../shared/agent-avatars/png";
const planes = new WeakMap<PixelPlane, HTMLCanvasElement>();
function planeCanvas(plane: PixelPlane) {
  let canvas = planes.get(plane);
  if (!canvas) {
    canvas = document.createElement("canvas");
    canvas.width = plane.width;
    canvas.height = plane.height;
    canvas
      .getContext("2d")!
      .putImageData(
        new ImageData(
          Uint8ClampedArray.from(plane.rgba),
          plane.width,
          plane.height,
        ),
        0,
        0,
      );
    planes.set(plane, canvas);
  }
  return canvas;
}
import type {
  DrawList,
  DrawPath,
} from "../../../../../shared/agent-avatars/draw";
// Static glyph/dither paths can be large. Reuse their parsed geometry across poses.
const parsedPaths = new WeakMap<DrawPath, Path2D>();
function parsedPath(part: DrawPath) {
  let path = parsedPaths.get(part);
  if (!path) {
    path = new Path2D(part.d);
    parsedPaths.set(part, path);
  }
  return path;
}
export function paintAvatar(
  context: CanvasRenderingContext2D,
  scene: DrawList,
  pixels = 512,
) {
  context.clearRect(0, 0, pixels, pixels);
  context.save();
  context.scale(pixels / 512, pixels / 512);
  context.beginPath();
  if (scene.frame === "circle") context.arc(256, 256, 256, 0, Math.PI * 2);
  else context.roundRect(0, 0, 512, 512, scene.frame === "tile" ? 96 : 0);
  context.clip();
  if (scene.backgroundPlane) {
    context.imageSmoothingEnabled = true;
    context.drawImage(planeCanvas(scene.backgroundPlane), 0, 0, 512, 512);
  }
  context.translate(scene.motion.x, scene.motion.y);
  context.translate(256, 256);
  context.rotate((scene.motion.rotate * Math.PI) / 180);
  context.scale(scene.motion.scale, scene.motion.scale);
  context.translate(-256, -256);
  const nums = scene.transform.match(/[-+]?\d*\.?\d+/g)!.map(Number);
  const matrix = new DOMMatrix([nums[2]!, 0, 0, nums[2]!, nums[0]!, nums[1]!]);
  const face = new Path2D();
  face.addPath(new Path2D(scene.mask), matrix);
  for (const p of scene.backgroundPaths) {
    context.globalAlpha = p.opacity ?? 1;
    context.fillStyle = p.fill;
    context.fill(parsedPath(p));
  }
  context.globalAlpha = 1;
  if (scene.foregroundPlane) {
    context.save();
    context.clip(face, "evenodd");
    context.drawImage(planeCanvas(scene.foregroundPlane), 0, 0, 512, 512);
    context.restore();
  }
  for (const p of scene.paths) {
    context.save();
    if (p.clip === "face") context.clip(face, "evenodd");
    if (p.clip === "outside")
      context.clip(new Path2D(scene.outsideMask), "evenodd");
    if (p.source)
      context.transform(matrix.a, 0, 0, matrix.d, matrix.e, matrix.f);
    context.globalAlpha = p.opacity ?? 1;
    const path = parsedPath(p);
    if (p.fill !== "none") {
      context.fillStyle = p.fill;
      context.fill(path);
    }
    if (p.stroke) {
      context.strokeStyle = p.stroke;
      context.lineWidth = p.width ?? 1;
      context.stroke(path);
    }
    context.restore();
  }
  context.restore();
}

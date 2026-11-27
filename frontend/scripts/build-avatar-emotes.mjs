/** Build a script-free runtime asset through the authenticated local Rive export. */
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";
import createRive from "@rive-app/canvas-advanced";
import { emoteSourceHash } from "./check-avatar-emotes.mjs";
import { validateRiveExport } from "./rive-export.mjs";
import { execPath } from "node:process";
const project = fileURLToPath(
  new URL("../../shared/agent-avatars/rive/", import.meta.url),
);
const cli =
  process.argv.find((value) => value.startsWith("--cli="))?.slice(6) ?? "rive";
const expectedWorkspace = process.argv
  .find((value) => value.startsWith("--workspace="))
  ?.slice(12);
execFileSync(execPath, [path.join(project, "generate-props.mjs"), "--check"], {
  stdio: "inherit",
});
const version = execFileSync(cli, ["--version"], { encoding: "utf8" }).trim();
const source = await readFile(path.join(project, "scene.rml"), "utf8");
if (/<(?:Script|Scripted|Shader)/.test(source))
  throw Error("This local export accepts script-free Rive artwork only.");
execFileSync(cli, [project, "--verify"], { stdio: "inherit" });
const inspection = JSON.parse(
  execFileSync(cli, ["inspect", project, "--summary"], { encoding: "utf8" }),
);
if (inspection.problems.length)
  throw Error(JSON.stringify(inspection.problems));
// Runtime export is distinct from preview transport and requires Rive sign-in.
// Run only after the author has selected the workspace whose allowance is used.
const report = JSON.parse(
  execFileSync(cli, [".", "--publish=local", "--format=json"], {
    cwd: project,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }),
);
const workspace = validateRiveExport(report, expectedWorkspace);
const bytes = await readFile(path.resolve(project, report.data.riv));
const runtime = await createRive({
  wasmBinary: await readFile(
    new URL(
      "../node_modules/@rive-app/canvas-advanced/rive.wasm",
      import.meta.url,
    ),
  ),
});
const file = await runtime.load(new Uint8Array(bytes));
const board = file.artboardByName("NyxEmotes");
if (!board || board.animationCount() !== 12)
  throw Error("Missing Rive emote timelines");
const values = file.viewModelByName("AvatarEmote").instance();
const machine = new runtime.StateMachineInstance(
  board.stateMachineByName("Emotes"),
  board,
);
machine.bindViewModelInstance(values);
const samples = [];
for (let emote = 0; emote < 12; emote++) {
  values.number("emote").value = emote;
  for (let frame = 0; frame < 45; frame++) {
    machine.advance(1 / 60);
    board.advance(1 / 60);
  }
  const eye = board.node("EyePose");
  const names = [
    "Keyboard",
    "TapLeft",
    "TapRight",
    "Tool",
    "Wrench",
    "Bolt",
    "Clock",
    "Minute",
    "Thought",
    "Request",
    "Retry",
    "Warning",
  ];
  const props = Object.fromEntries(
    names.map((name) => {
      const node = board.node(name);
      if (!node) throw Error(`Missing prop node ${name}`);
      return [
        name,
        {
          x: node.x,
          y: node.y,
          rotation: node.rotation,
          scaleX: node.scaleX,
          scaleY: node.scaleY,
        },
      ];
    }),
  );
  samples.push({ emote, x: eye.x, y: eye.y, openness: eye.scaleY, props });
}
const hash = (value) => createHash("sha256").update(value).digest("hex");
await writeFile(path.join(project, "nyx-emotes.riv"), bytes);
await writeFile(
  path.join(project, "manifest.json"),
  JSON.stringify(
    {
      version: 1,
      compiler: version,
      exportMethod: "authenticated-local",
      workspace,
      artboard: "NyxEmotes",
      stateMachine: "Emotes",
      dataModel: "AvatarEmote",
      binding: "emote",
      sourceSha256: await emoteSourceHash(),
      assetSha256: hash(bytes),
      bytes: bytes.length,
      samples,
    },
    null,
    2,
  ) + "\n",
);
machine.delete();
board.delete();
values.delete();
file.delete();
console.log(
  `Verified ${samples.length} Rive emotes; wrote ${bytes.length} bytes locally.`,
);

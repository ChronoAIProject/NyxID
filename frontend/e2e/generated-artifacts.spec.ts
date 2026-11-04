import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { openAssistant, sendMessage } from "./helpers";

async function installArtifacts(
  page: Page,
  profile: "baseline" | "high" | "fragmented" = "baseline",
) {
  await page.route("**/api/v1/assistant/nyxagent/machines?*", (route) =>
    route.fulfill({ json: [] }),
  );
  const movie = await readFile(
    new URL(
      `../../backend/tests/fixtures/generated-artifacts/visual-${profile === "baseline" ? "avc" : profile}.mp4`,
      import.meta.url,
    ),
  );
  const png = await readFile(
    new URL(
      "../../backend/tests/fixtures/generated-artifacts/blue.png",
      import.meta.url,
    ),
  );
  await page.addInitScript(
    ({ video, image }) => {
      let original: typeof globalThis.__nyxidAssistantHttpMock;
      Object.defineProperty(globalThis, "__nyxidAssistantHttpMock", {
        configurable: true,
        get: () => async (request: { endpoint: string; init: RequestInit }) => {
          const type = request.endpoint.endsWith("/artifact-video2037")
            ? "video/mp4"
            : request.endpoint.endsWith("/artifact-image2037")
              ? "image/png"
              : null;
          if (type) {
            const bytes = Uint8Array.from(
              atob(type === "video/mp4" ? video : image),
              (c) => c.charCodeAt(0),
            );
            return new Response(bytes, {
              headers: {
                "content-type": type,
                "cache-control": "private, no-store",
              },
            });
          }
          return original?.(request);
        },
        set: (handler) => {
          original = handler;
        },
      });
    },
    { video: movie.toString("base64"), image: png.toString("base64") },
  );
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Create a small cloud video");
  await expect(
    page
      .getByText("Your connected services are ready.", { exact: false })
      .first(),
  ).toBeVisible();
  await page.evaluate(
    ({ videoSize, imageSize, videoPreview }) => {
      const state = JSON.parse(
        sessionStorage.getItem("nyxagent-http-fixture-v4")!,
      );
      const id = new URLSearchParams(location.search).get("c");
      const row = state.rows.find(([key]: [string]) => key === id)[1];
      const message = row.history.messages.at(-1);
      message.text = "The verified files are attached to this reply.";
      message.attachments = [
        {
          id: "artifact-video2037",
          filename: "movie.mp4",
          content_type: "video/mp4",
          preview_kind: videoPreview,
          size: videoSize,
        },
        {
          id: "artifact-image2037",
          filename: "blue.png",
          content_type: "image/png",
          preview_kind: "image",
          size: imageSize,
        },
      ].map((item) => ({
        ...item,
        label: "Sandbox result",
        origin: "generated_artifact",
        source: "sandbox",
        result_version: "1",
        expires_at: new Date(Date.now() + 3600000).toISOString(),
      }));
      message.artifact_delivery = {
        state: "materialized",
        count: 2,
        recovery_available: false,
      };
      sessionStorage.setItem("nyxagent-http-fixture-v4", JSON.stringify(state));
    },
    {
      videoSize: movie.length,
      imageSize: png.length,
      videoPreview: profile === "fragmented" ? "download" : "video",
    },
  );
  await page.reload();
  return { movie, png };
}

for (const profile of ["baseline", "high"] as const) {
  test(`private ${profile} MP4 plays, pauses, downloads byte-identically and survives revisit with image regression`, async ({
    page,
  }, info) => {
    const { movie, png } = await installArtifacts(page, profile);
    const video = page.getByLabel("Preview movie.mp4", { exact: true });
    await expect(video).toBeVisible();
    await expect
      .poll(() =>
        video.evaluate((element) => (element as HTMLVideoElement).readyState),
      )
      .toBeGreaterThanOrEqual(2);
    await video.evaluate(async (element) => {
      await (element as HTMLVideoElement).play();
    });
    await expect
      .poll(() =>
        video.evaluate((element) => (element as HTMLVideoElement).currentTime),
      )
      .toBeGreaterThan(0);
    await video.evaluate((element) => (element as HTMLVideoElement).pause());
    expect(
      await video.evaluate((element) => ({
        paused: (element as HTMLVideoElement).paused,
        error: (element as HTMLVideoElement).error,
      })),
    ).toEqual({ paused: true, error: null });
    const download = page.getByRole("link", {
      name: "Download movie.mp4",
      exact: true,
    });
    await download.focus();
    const downloaded = page.waitForEvent("download");
    await page.keyboard.press("Enter");
    const file = await downloaded;
    const filename = info.outputPath("downloaded-video.mp4");
    await file.saveAs(filename);
    expect(await readFile(filename)).toEqual(movie);
    const probe = JSON.parse(
      execFileSync(
        "ffprobe",
        [
          "-v",
          "error",
          "-show_entries",
          "stream=codec_name,codec_type,profile",
          "-of",
          "json",
          filename,
        ],
        { encoding: "utf8" },
      ),
    );
    expect(probe.streams).toContainEqual({
      codec_name: "h264",
      codec_type: "video",
      profile: profile === "high" ? "High" : "Constrained Baseline",
    });
    const image = page.getByAltText("blue.png", { exact: true });
    await expect(image).toHaveJSProperty("naturalWidth", 16);
    const source = await image.getAttribute("src");
    await expect(
      page.getByRole("link", { name: "Open full preview of blue.png" }),
    ).toHaveAttribute("href", source!);
    const imageDownload = page.waitForEvent("download");
    await page.getByRole("link", { name: "Download blue.png" }).click();
    const imageFile = await imageDownload;
    const imagePath = info.outputPath("downloaded-image.png");
    await imageFile.saveAs(imagePath);
    expect(await readFile(imagePath)).toEqual(png);
    await page.screenshot({ path: info.outputPath("generated-files.png") });
    await page.reload();
    await expect(video).toBeVisible();
    await expect(image).toHaveJSProperty("naturalWidth", 16);
  });
}

test("fragmented MP4 stays download-only and downloads the exact verified bytes", async ({
  page,
}) => {
  const { movie } = await installArtifacts(page, "fragmented");
  await expect(
    page.getByLabel("Preview movie.mp4", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText(
      "Preview unavailable for this type. Download the file to open it.",
    ),
  ).toBeVisible();
  const downloaded = page.waitForEvent("download");
  await page
    .getByRole("button", { name: "Download movie.mp4", exact: true })
    .click();
  const file = await downloaded;
  expect(await readFile((await file.path())!)).toEqual(movie);
});

test("browser decoder failure removes playback while the exact download remains usable", async ({
  page,
}) => {
  const { movie } = await installArtifacts(page);
  const video = page.getByLabel("Preview movie.mp4", { exact: true });
  await expect(video).toBeVisible();
  await video.evaluate((element) => element.dispatchEvent(new Event("error")));
  await expect(video).toHaveCount(0);
  await expect(
    page.getByText(
      "Your browser could not decode this preview. You can still download the verified file.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Download movie.mp4", exact: true }),
  ).toBeVisible();
  const downloaded = page.waitForEvent("download");
  await page
    .getByRole("link", { name: "Download movie.mp4", exact: true })
    .click();
  const file = await downloaded;
  expect(await readFile((await file.path())!)).toEqual(movie);
});

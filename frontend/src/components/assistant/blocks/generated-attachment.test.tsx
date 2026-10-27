import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ToolImage } from "./tool-image";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import type { ChatImage } from "@/lib/assistant/chat-types";
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantHttp: vi.fn() }));
const video: ChatImage = {
  id: "artifact2037",
  origin: "generated_artifact",
  endpoint: "/assistant/nyxagent/conversations/chat/attachments/artifact2037",
  contentType: "video/mp4",
  label: "Sandbox result",
  filename: "movie.mp4",
  size: 1024,
  previewKind: "video",
  resultVersion: "1",
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:private2037"),
    revokeObjectURL: vi.fn(),
  });
  vi.mocked(assistantHttp).mockResolvedValue(
    new Response(new Blob(["fixture"])),
  );
});
afterEach(() => vi.unstubAllGlobals());
it("uses one private source for video playback and download, revokes URLs and falls back on decoder error", async () => {
  const { unmount } = render(<ToolImage image={video} />);
  expect(screen.getByRole("status")).toHaveTextContent("Loading");
  const preview = await screen.findByLabelText("Preview movie.mp4");
  expect(preview.tagName).toBe("VIDEO");
  expect(preview).toHaveAttribute("controls");
  const download = screen.getByRole("link", { name: "Download movie.mp4" });
  expect(download).toHaveAttribute("href", preview.getAttribute("src"));
  expect(download).toHaveAttribute("download", "movie.mp4");
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  expect(screen.getByText(/1,024 bytes · Revision 1/)).toBeInTheDocument();
  fireEvent.error(preview);
  expect(screen.queryByLabelText("Preview movie.mp4")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("could not decode");
  expect(download).toHaveAttribute("href", "blob:private2037");
  unmount();
  expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:private2037");
});
it("preserves generated image thumbnail/full preview/download on the same attachment", async () => {
  render(
    <ToolImage
      image={{
        ...video,
        filename: "image.png",
        contentType: "image/png",
        previewKind: "image",
      }}
    />,
  );
  const image = await screen.findByAltText("image.png");
  expect(
    screen.getByRole("link", { name: "Open full preview of image.png" }),
  ).toHaveAttribute("href", image.getAttribute("src"));
  expect(
    screen.getByRole("link", { name: "Download image.png" }),
  ).toHaveAttribute("href", image.getAttribute("src"));
  expect(assistantHttp).toHaveBeenCalledWith(
    video.endpoint,
    expect.objectContaining({ signal: expect.any(AbortSignal) }),
  );
});
it("offers downloads for unsupported types without a play control", async () => {
  const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  const { unmount } = render(
    <ToolImage
      image={{
        ...video,
        contentType: "application/octet-stream",
        filename: "report.html",
        previewKind: "download",
      }}
    />,
  );
  expect(assistantHttp).not.toHaveBeenCalled();
  expect(URL.createObjectURL).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toHaveTextContent("Preview unavailable for this type");
  fireEvent.click(screen.getByRole("button", { name: "Download report.html" }));
  expect(screen.getByRole("status")).toHaveTextContent("Loading");
  const download = await screen.findByRole("link", { name: "Download report.html" });
  expect(download).toHaveAttribute("href", "blob:private2037");
  expect(download).toHaveAttribute("download", "report.html");
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  expect(assistantHttp).toHaveBeenCalledWith(video.endpoint, expect.objectContaining({ signal: expect.any(AbortSignal) }));
  expect(click).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("status")).toHaveTextContent(
    "Preview unavailable for this type",
  );
  expect(screen.queryByLabelText(/Preview report/)).not.toBeInTheDocument();
  unmount();
  expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:private2037");
  click.mockRestore();
});
it.each([
  [410, 12101, "expired"],
  [404, 404, "not found"],
  [403, 403, "not authorized"],
])("distinguishes owner read failures %s", async (status, code, copy) => {
  vi.mocked(assistantHttp).mockRejectedValue(
    new ApiError(status as number, {
      error: "unavailable",
      error_code: code as number,
      message: "unavailable",
    }),
  );
  render(<ToolImage image={video} />);
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent(copy as string),
  );
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
});
it("does not fetch retained-expiry metadata and cancels a pending request on unmount", () => {
  const expired = render(<ToolImage image={{ ...video, expired: true }} />);
  expect(screen.getByRole("status")).toHaveTextContent("expired");
  expect(assistantHttp).not.toHaveBeenCalled();
  expired.unmount();
  vi.mocked(assistantHttp).mockReturnValue(new Promise(() => {}));
  const pending = render(<ToolImage image={video} />);
  const signal = vi.mocked(assistantHttp).mock.calls[0]![1]!.signal!;
  pending.unmount();
  expect(signal.aborted).toBe(true);
});

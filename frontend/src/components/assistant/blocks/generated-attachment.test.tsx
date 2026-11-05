import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GeneratedAttachment } from "./generated-attachment";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { clearGeneratedAttachmentCache } from "@/lib/assistant/generated-attachment-cache";
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantHttp: vi.fn() }));
const file = {
  id: "movie",
  origin: "generated_artifact",
  endpoint: "/private/movie",
  contentType: "video/mp4",
  label: "Sandbox",
  filename: "movie.mp4",
  previewKind: "video",
  size: 42,
  resultVersion: "2",
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:private"),
    revokeObjectURL: vi.fn(),
  });
  vi.mocked(assistantHttp).mockImplementation(
    async () => new Response("video bytes"),
  );
});
afterEach(() => {
  clearGeneratedAttachmentCache();
  vi.unstubAllGlobals();
});
it("renders supported video controls and a download when preview decoding fails", async () => {
  render(<GeneratedAttachment attachment={file} />);
  const video = await screen.findByLabelText("Preview movie.mp4");
  expect(video).toHaveAttribute("controls");
  expect(screen.getByText(/Revision 2/)).toBeInTheDocument();
  fireEvent.error(video);
  expect(screen.getByText(/could not decode/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Download/ })).toHaveAttribute(
    "download",
    "movie.mp4",
  );
});
it("falls back safely for future preview, MIME and origin values without fetching on mount", () => {
  const view = render(
    <GeneratedAttachment attachment={{ ...file, previewKind: "future" }} />,
  );
  expect(screen.getByText(/Preview unavailable/)).toBeInTheDocument();
  view.rerender(
    <GeneratedAttachment
      attachment={{ ...file, contentType: "video/future" }}
    />,
  );
  view.rerender(
    <GeneratedAttachment attachment={{ ...file, origin: "future" }} />,
  );
  expect(assistantHttp).not.toHaveBeenCalled();
});
it("does not fetch an expired file", () => {
  render(<GeneratedAttachment attachment={{ ...file, expired: true }} />);
  expect(screen.getByText(/has expired/)).toBeInTheDocument();
  expect(assistantHttp).not.toHaveBeenCalled();
});

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ToolImage } from "./tool-image";
import { assistantHttp } from "@/lib/assistant/assistant-http";
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantHttp: vi.fn() }));
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:private"),
    revokeObjectURL: vi.fn(),
  });
  vi.mocked(assistantHttp).mockResolvedValue(new Response(new Blob(["bytes"])));
});
afterEach(() => vi.unstubAllGlobals());
it("fetches an owner image and preserves the explicit agent-version fallback", async () => {
  const { unmount } = render(
    <ToolImage
      image={{
        id: "photo",
        endpoint: "/private/photo",
        contentType: "image/png",
        label: "photo.png",
        imageInput: "unavailable",
      }}
    />,
  );
  expect(await screen.findByAltText("Image from photo.png")).toHaveAttribute(
    "src",
    "blob:private",
  );
  expect(screen.getByRole("status")).toHaveTextContent("cannot view");
  expect(assistantHttp).toHaveBeenCalledWith("/private/photo");
  unmount();
  expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:private");
});
it("downloads documents only after an owner click and reports expiry", async () => {
  vi.mocked(assistantHttp).mockRejectedValue(new Error("Expired"));
  render(
    <ToolImage
      image={{
        id: "doc",
        endpoint: "/private/doc",
        contentType: "application/pdf",
        label: "report.pdf",
      }}
    />,
  );
  expect(assistantHttp).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /report.pdf/ }));
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent("expired"),
  );
  expect(assistantHttp).toHaveBeenCalledWith("/private/doc");
});

it.each(["image/png", "application/pdf"])("shows a retention placeholder for expired %s without fetching bytes", (contentType) => {
  render(<ToolImage image={{ id: "expired", endpoint: "/private/expired", label: "old file", contentType, expired: true }} />);
  expect(screen.getByRole("status")).toHaveTextContent("expired per retention policy");
  expect(assistantHttp).not.toHaveBeenCalled();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it("handles retention expiry between history and the image download", async () => {
  const { ApiError } = await import("@/lib/api-client");
  vi.mocked(assistantHttp).mockRejectedValue(new ApiError(410, { error: "attachment_expired", error_code: 12101, message: "Attachment expired per retention policy." }));
  render(<ToolImage image={{ id: "expired", endpoint: "/private/expired", label: "old image", contentType: "image/png" }} />);
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("expired per retention policy"));
});

it.each([undefined, "sent", "unavailable"] as const)("renders tool image delivery=%s without upload advice", async (imageInput) => {
  render(<ToolImage image={{ id: "camera", endpoint: "/private/camera", contentType: "image/png", label: "camera snapshot", origin: "tool", imageInput }} />);
  await screen.findByAltText("Image from camera snapshot");
  expect(screen.queryByText(/try a smaller image/)).not.toBeInTheDocument();
  if (imageInput === "unavailable") expect(screen.getByRole("status")).toHaveTextContent("The agent couldn't view this image.");
  else expect(screen.queryByRole("status")).not.toBeInTheDocument();
});


it.each(["model_text_only", "image_input_unsupported"])("explains %s without machine or size advice", async (imageInputReason) => {
  render(<ToolImage image={{ id: "photo", endpoint: "/private/photo", contentType: "image/png", label: "photo.png", origin: "user_upload", imageInput: "unavailable", imageInputReason }} />);
  await screen.findByAltText("Image from photo.png");
  expect(screen.getByRole("status")).toHaveTextContent("This model can’t view images.");
  expect(screen.getByRole("status")).not.toHaveTextContent(/machine|smaller/);
});

it("shows retained guest images with an honest model notice and no tool suggestion", async () => {
  render(<ToolImage image={{ id: "guest-photo", endpoint: "/private/guest-photo", contentType: "image/png", label: "guest.png", origin: "user_upload", imageInput: "unavailable", imageInputReason: "guest_media_unavailable" }} />);
  await screen.findByAltText("Image from guest.png");
  expect(screen.getByRole("status")).toHaveTextContent("unavailable to the model in that turn");
  expect(screen.getByRole("status")).not.toHaveTextContent(/machine|smaller/);
});

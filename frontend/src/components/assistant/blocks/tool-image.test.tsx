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

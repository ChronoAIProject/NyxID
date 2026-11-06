import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  vi.stubGlobal("IntersectionObserver", undefined);
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
  vi.restoreAllMocks();
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

function observeVisibility() {
  const observed = new Map<Element, (visible: boolean) => void>();
  vi.stubGlobal("IntersectionObserver", class {
    readonly callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
    }
    observe(element: Element) {
      observed.set(element, (visible) => this.callback(
        [{
          target: element,
          isIntersecting: visible,
          boundingClientRect: { height: 320 },
        } as IntersectionObserverEntry],
        this as unknown as IntersectionObserver,
      ));
    }
    disconnect = vi.fn();
  });
  return observed;
}

it("releases offscreen cards so every preview in an eight-file reply can load", async () => {
  const observed = observeVisibility();
  render(<>{Array.from({ length: 8 }, (_, n) => (
    <GeneratedAttachment key={n} attachment={{ ...file, id: String(n), endpoint: `/private/${n}`, filename: `movie-${n}.mp4` }} />
  ))}</>);
  expect(assistantHttp).not.toHaveBeenCalled();
  const cards = [...observed.values()];
  act(() => cards.forEach((show) => show(true)));
  await screen.findByLabelText("Preview movie-7.mp4");
  expect(assistantHttp).toHaveBeenCalledTimes(8);
  act(() => {
    cards.slice(0, 4).forEach((show) => show(false));
    cards.slice(4).forEach((show) => show(true));
  });
  await screen.findByLabelText("Preview movie-7.mp4");
  expect(assistantHttp).toHaveBeenCalledTimes(8);
  expect(screen.queryByLabelText("Preview movie-0.mp4")).not.toBeInTheDocument();
  expect([...observed.keys()][0]).toHaveStyle({ minHeight: "320px" });
  act(() => {
    cards.slice(4).forEach((show) => show(false));
    cards[0]!(true);
  });
  await waitFor(() => expect(screen.getByLabelText("Preview movie-0.mp4")).toBeInTheDocument());
});

it("restores an offscreen download link without automatically downloading again", async () => {
  const observed = observeVisibility();
  const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  render(<GeneratedAttachment attachment={{ ...file, previewKind: "download", contentType: "application/octet-stream" }} />);
  const show = [...observed.values()][0]!;
  act(() => show(true));
  fireEvent.click(screen.getByRole("button", { name: /Download/ }));
  await waitFor(() => expect(click).toHaveBeenCalledTimes(1));
  act(() => show(false));
  clearGeneratedAttachmentCache();
  act(() => show(true));
  await screen.findByRole("link", { name: /Download/ });
  expect(assistantHttp).toHaveBeenCalledTimes(2);
  expect(click).toHaveBeenCalledTimes(1);
});

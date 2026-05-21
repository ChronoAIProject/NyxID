import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mockNavigate = vi.fn();

// DocsSearch uses both useNavigate (Enter to open) and Link (rendered rows).
// The Link mock turns `to="/docs/$"` + `params._splat` into a real href and
// forwards the click/hover handlers the component attaches.
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  Link: ({ to, params, children, ...rest }: any) => (
    <a href={`/docs/${params?._splat ?? ""}`} data-to={to} {...rest}>
      {children}
    </a>
  ),
}));

import { DocsSearch } from "./docs-search";

// All four entries contain "service" so the query returns them in index order.
const INDEX = [
  { source: "cli/reference/service.md", title: "nyxid service", description: "manage services", headings: ["service add"] },
  { source: "cli/guides/connect-a-service.md", title: "Connect an AI service", description: "connect a service", headings: [] },
  { source: "web/guides/manage-keys.md", title: "Manage keys", description: "keys and services", headings: [] },
  { source: "shared/concepts/the-proxy.md", title: "The proxy", description: "route a service", headings: [] },
];

const SLUGS = INDEX.map((e) => e.source.replace(/\.md$/, ""));

function selectedOptionHref(): string | null {
  const opt = screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true");
  return opt ? within(opt).getByRole("link").getAttribute("href") : null;
}

describe("DocsSearch keyboard navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // happy-dom doesn't implement scrollIntoView; the scroll-into-view effect
    // calls it on every selection change.
    Element.prototype.scrollIntoView = vi.fn();
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve({ ok: true, json: () => Promise.resolve(INDEX) })),
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  async function openAndSearch() {
    const user = userEvent.setup();
    render(<DocsSearch open onClose={vi.fn()} />);
    const input = await screen.findByRole("combobox");
    await user.type(input, "service");
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));
    return user;
  }

  it("highlights the first result by default", async () => {
    await openAndSearch();
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[0]}`);
    // The combobox points aria-activedescendant at the highlighted option.
    expect(screen.getByRole("combobox")).toHaveAttribute("aria-activedescendant", "docs-search-opt-0");
  });

  it("moves the highlight down and up with the arrow keys", async () => {
    const user = await openAndSearch();
    await user.keyboard("{ArrowDown}");
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[1]}`);
    await user.keyboard("{ArrowDown}");
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[2]}`);
    await user.keyboard("{ArrowUp}");
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[1]}`);
  });

  it("wraps around at both ends", async () => {
    const user = await openAndSearch();
    await user.keyboard("{ArrowUp}"); // from first → last
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[3]}`);
    await user.keyboard("{ArrowDown}"); // last → first
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[0]}`);
  });

  it("opens the highlighted result on Enter and closes the palette", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<DocsSearch open onClose={onClose} />);
    const input = await screen.findByRole("combobox");
    await user.type(input, "service");
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(4));

    await user.keyboard("{ArrowDown}{Enter}"); // open the 2nd result
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/docs/$", params: { _splat: SLUGS[1] } });
    expect(onClose).toHaveBeenCalled();
  });

  it("syncs the highlight to the hovered result", async () => {
    const user = await openAndSearch();
    const options = screen.getAllByRole("option");
    await user.hover(within(options[2]!).getByRole("link"));
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[2]}`);
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<DocsSearch open onClose={onClose} />);
    await screen.findByRole("combobox");
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("resets the highlight to the top when the query changes", async () => {
    const user = await openAndSearch();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(selectedOptionHref()).toBe(`/docs/${SLUGS[2]}`);
    // Narrow the query — selection should jump back to the first match.
    await user.type(screen.getByRole("combobox"), "s");
    await waitFor(() => expect(selectedOptionHref()).toBe(`/docs/${SLUGS[0]}`));
  });
});

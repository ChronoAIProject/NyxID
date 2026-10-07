import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ServiceIcon } from "./service-icon";

describe("ServiceIcon", () => {
  it.each([
    ["cloudflare", "api-cloudflare"],
    ["api-cloudflare", "api-cloudflare"],
    ["supabase-management", "api-supabase-management"],
    ["api-supabase-management", "api-supabase-management"],
    ["railway", "api-railway"],
    ["api-railway", "api-railway"],
  ])("renders the brand SVG for %s", (slug, catalogSlug) => {
    const { container } = render(<ServiceIcon slug={slug} size="md" />);
    const svg = container.querySelector(`svg[data-slug="${catalogSlug}"]`);
    expect(svg).toBeInTheDocument();
    expect(svg).toHaveAttribute("fill", "currentColor");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).toHaveClass("!h-6", "!w-6");
    expect(container.querySelector('[data-fallback="true"]')).toBeNull();
  });

  it("uses a user icon URL and falls back to the built-in glyph if it fails", () => {
    const { container } = render(
      <ServiceIcon slug="cma" iconUrl="https://example.com/cma.svg" size="md" />,
    );
    const image = container.querySelector("img");
    expect(image).not.toBeNull();
    if (!image) return;
    expect(image).toHaveAttribute("src", "https://example.com/cma.svg");
    fireEvent.error(image);
    expect(container.querySelector('[data-slug="cma"]')).toBeInTheDocument();
  });

  it("renders an image for a custom service without a catalog slug", () => {
    const { container } = render(<ServiceIcon iconUrl="https://example.com/custom.png" />);
    expect(container.querySelector("img")).toHaveAttribute(
      "src",
      "https://example.com/custom.png",
    );
  });

  it.each([
    "javascript:alert(1)",
    "data:image/svg+xml,<svg onload=alert(1) />",
    "https://user:secret@example.com/icon.svg",
    "https://example.com/icon.svg#fragment",
    "not a URL",
  ])("falls back to the glyph for an unsafe icon URL: %s", (iconUrl) => {
    const { container } = render(<ServiceIcon slug="cma" iconUrl={iconUrl} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('[data-slug="cma"]')).toBeInTheDocument();
  });
});

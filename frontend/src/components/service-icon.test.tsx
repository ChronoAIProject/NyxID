import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
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

  it("renders uploaded images greyscale, lifted in dark themes, like the glyphs", () => {
    const { container } = render(
      <ServiceIcon slug="api-zoho-mail" iconUrl="https://example.com/zoho.svg" size="sm" />,
    );
    const image = container.querySelector("img");
    expect(image).toHaveClass("grayscale", "dark:brightness-150", "!h-5", "!w-5");
    expect(image).toHaveAttribute("referrerpolicy", "no-referrer");
  });

  it("falls back to the generic glyph when an uploaded image fails for an unknown slug", () => {
    const { container } = render(
      <ServiceIcon slug="api-zoho-mail" iconUrl="https://example.com/zoho.svg" />,
    );
    const image = container.querySelector("img");
    expect(image).not.toBeNull();
    if (!image) return;
    fireEvent.error(image);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('[data-fallback="true"]')).toBeInTheDocument();
  });

  describe("glyph-coloured mask upgrade", () => {
    interface Probe {
      onload: (() => void) | null;
      onerror: (() => void) | null;
      crossOrigin: string | null;
      src: string;
    }
    const probes: Probe[] = [];
    afterEach(() => {
      vi.unstubAllGlobals();
      probes.length = 0;
    });

    function stubImage() {
      vi.stubGlobal(
        "Image",
        class {
          onload: (() => void) | null = null;
          onerror: (() => void) | null = null;
          crossOrigin: string | null = null;
          referrerPolicy = "";
          src = "";
          constructor() {
            probes.push(this);
          }
        },
      );
    }

    it("switches to a mask in the glyph colour once the image is CORS-readable", async () => {
      stubImage();
      const url = "https://nyx.example/catalog-icons/zoho";
      const { container } = render(<ServiceIcon slug="api-zoho-mail" iconUrl={url} size="sm" />);
      expect(container.querySelector('[data-icon-mode="image"]')).not.toBeNull();
      expect(probes[0]?.crossOrigin).toBe("anonymous");
      expect(probes[0]?.src).toBe(url);
      await act(async () => probes[0]?.onload?.());
      const mask = container.querySelector('[data-icon-mode="mask"]');
      expect(mask).not.toBeNull();
      expect(mask).toHaveClass("bg-current", "text-muted-foreground", "!h-5", "!w-5");
      expect(mask).toHaveAttribute("aria-hidden", "true");
      expect(container.querySelector("img")).toBeNull();
      const style = mask?.getAttribute("style") ?? "";
      if (style.includes("mask")) expect(style).toContain(`url("${url}")`);
    });

    it("remembers a readable URL so later mounts start as a mask without a new probe", async () => {
      stubImage();
      const url = "https://nyx.example/catalog-icons/cached";
      const first = render(<ServiceIcon slug="api-zoho-mail" iconUrl={url} />);
      await act(async () => probes[0]?.onload?.());
      first.unmount();
      const second = render(<ServiceIcon slug="api-zoho-mail" iconUrl={url} />);
      expect(second.container.querySelector('[data-icon-mode="mask"]')).not.toBeNull();
      expect(probes).toHaveLength(1);
    });

    it("ignores a probe that finishes after unmount", async () => {
      stubImage();
      const url = "https://nyx.example/catalog-icons/unmounted";
      const { unmount } = render(<ServiceIcon slug="api-zoho-mail" iconUrl={url} />);
      unmount();
      await act(async () => probes[0]?.onload?.());
      const again = render(<ServiceIcon slug="api-zoho-mail" iconUrl={url} />);
      expect(again.container.querySelector('[data-icon-mode="mask"]')).not.toBeNull();
    });

    it("keeps the greyscale image when the host does not allow CORS", async () => {
      stubImage();
      const { container } = render(
        <ServiceIcon slug="api-aurinko" iconUrl="https://vendor.example/icon.svg" />,
      );
      await act(async () => probes[0]?.onerror?.());
      expect(container.querySelector('[data-icon-mode="image"]')).toHaveClass("grayscale");
      expect(container.querySelector('[data-icon-mode="mask"]')).toBeNull();
    });
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

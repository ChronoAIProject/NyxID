import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { StepHeader } from "./step-header";

function renderHeader(iconUrl?: string | null) {
  return render(
    <Dialog open>
      <DialogContent>
        <StepHeader slug="api-zoho-mail" iconUrl={iconUrl} title="Zoho Mail" />
      </DialogContent>
    </Dialog>,
  );
}

describe("StepHeader", () => {
  it("shows the uploaded catalog icon when one exists", () => {
    renderHeader("https://nyx.example/catalog-icons/zoho");
    const image = document.querySelector("img");
    expect(image).toHaveAttribute("src", "https://nyx.example/catalog-icons/zoho");
  });

  it("falls back to the slug glyph without an icon", () => {
    renderHeader(null);
    expect(document.querySelector("img")).toBeNull();
    expect(document.querySelector('[data-fallback="true"]')).toBeInTheDocument();
  });
});

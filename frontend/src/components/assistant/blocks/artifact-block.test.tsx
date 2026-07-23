import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ArtifactBlock } from "./artifact-block";

/** The inert-download branch renders a Radix Tooltip, which needs a provider. */
function renderBlock(node: React.ReactElement) {
  return render(<TooltipProvider>{node}</TooltipProvider>);
}

describe("ArtifactBlock download affordance", () => {
  it("links to download_url when the block carries one", () => {
    renderBlock(
      <ArtifactBlock
        block={{
          type: "artifact",
          block_id: "a1",
          artifact_id: "art-1",
          name: "digest.md",
          mime: "text/markdown",
          size_bytes: 120,
          preview: null,
          download_url: "https://aevatar.example/artifacts/art-1",
        }}
      />,
    );

    const link = screen.getByRole("link", { name: /Download digest\.md/ });
    expect(link).toHaveAttribute(
      "href",
      "https://aevatar.example/artifacts/art-1",
    );
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("keeps the control inert, not broken, when there is no URL", () => {
    renderBlock(
      <ArtifactBlock
        block={{
          type: "artifact",
          block_id: "a1",
          artifact_id: "art-1",
          name: "digest.md",
          mime: "text/markdown",
          size_bytes: 120,
          preview: null,
          download_url: "",
        }}
      />,
    );

    expect(
      screen.getByRole("button", { name: /Download digest\.md/ }),
    ).toBeDisabled();
  });
});

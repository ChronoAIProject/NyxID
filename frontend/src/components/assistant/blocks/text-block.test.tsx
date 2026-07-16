import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TextBlock } from "./text-block";

describe("TextBlock", () => {
  it("renders the supported markdown subset", () => {
    render(
      <TextBlock text={"**Scoped** and *brokered* with `charges:read`."} />,
    );

    expect(screen.getByText("Scoped").tagName).toBe("STRONG");
    expect(screen.getByText("brokered").tagName).toBe("EM");
    expect(screen.getByText("charges:read").tagName).toBe("CODE");
  });

  it("keeps raw HTML inert and never renders remote images", () => {
    const { container } = render(
      <TextBlock
        text={
          '<script>alert("x")</script>\n![remote](https://example.com/pixel.png)'
        }
      />,
    );

    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("img")).not.toBeInTheDocument();
    expect(container).toHaveTextContent('<script>alert("x")</script>');
  });

  it("allows only https and mailto links with safe rel attributes", () => {
    render(
      <TextBlock
        text={[
          "[Secure](https://nyxid.dev)",
          "[Email](mailto:security@nyxid.dev)",
          "[Plain HTTP](http://nyxid.dev)",
          "[Script](javascript:alert(1))",
        ].join(" ")}
      />,
    );

    expect(screen.getByRole("link", { name: "Secure" })).toHaveAttribute(
      "rel",
      "noopener noreferrer",
    );
    expect(screen.getByRole("link", { name: "Email" })).toHaveAttribute(
      "href",
      "mailto:security@nyxid.dev",
    );
    expect(
      screen.queryByRole("link", { name: "Plain HTTP" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "Script" }),
    ).not.toBeInTheDocument();
  });
});

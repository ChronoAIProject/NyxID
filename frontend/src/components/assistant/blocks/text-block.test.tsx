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
    expect(screen.getByRole("link", { name: "remote" })).toHaveAttribute(
      "href",
      "https://example.com/pixel.png",
    );
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

  it("accepts allowed schemes case-insensitively and keeps fragments local", () => {
    render(
      <TextBlock
        text={"[Secure](HTTPS://nyxid.dev) [Footnote](#fn-1)"}
      />,
    );

    expect(screen.getByRole("link", { name: "Secure" })).toHaveAttribute(
      "href",
      "HTTPS://nyxid.dev",
    );
    expect(screen.getByRole("link", { name: "Footnote" })).toHaveAttribute(
      "href",
      "#fn-1",
    );
    expect(screen.getByRole("link", { name: "Footnote" })).not.toHaveAttribute(
      "target",
    );
  });

  it("renders GFM tables and strikethrough", () => {
    const { container } = render(
      <TextBlock
        text={[
          "| Service | Scope |",
          "| :--- | ---: |",
          "| Stripe | ~~write~~ read |",
        ].join("\n")}
      />,
    );

    expect(
      screen.getByRole("columnheader", { name: "Service" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "Stripe" })).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "Service" }),
    ).toHaveStyle({ textAlign: "left" });
    expect(screen.getByRole("columnheader", { name: "Scope" })).toHaveStyle({
      textAlign: "right",
    });
    expect(screen.getByText("write").tagName).toBe("DEL");
    expect(container.querySelector("table")).toBeInTheDocument();
  });

  it("renders task-list checkboxes as disabled controls", () => {
    render(<TextBlock text={"- [x] Brokered\n- [ ] Pending"} />);

    const checkboxes = screen.getAllByRole("checkbox");
    expect(checkboxes).toHaveLength(2);
    expect(checkboxes[0]).toBeChecked();
    expect(checkboxes[0]).toBeDisabled();
    expect(checkboxes[1]).not.toBeChecked();
    expect(checkboxes[1]).toBeDisabled();
  });

  it("autolinks only allowed GFM URL protocols", () => {
    render(
      <TextBlock
        text={"Open https://nyxid.dev or http://unsafe.example now."}
      />,
    );

    expect(
      screen.getByRole("link", { name: "https://nyxid.dev" }),
    ).toHaveAttribute("href", "https://nyxid.dev");
    expect(
      screen.queryByRole("link", { name: "http://unsafe.example" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/http:\/\/unsafe\.example/)).toBeInTheDocument();
  });

  it("renders CommonMark angle autolinks through the same URL policy", () => {
    render(<TextBlock text={"<https://nyxid.dev/security>"} />);

    expect(
      screen.getByRole("link", { name: "https://nyxid.dev/security" }),
    ).toHaveAttribute("href", "https://nyxid.dev/security");
  });

  it("preserves angle brackets inside fenced and inline code", () => {
    const { container } = render(
      <TextBlock
        text={[
          "```tsx",
          "const scopes: Vec<String> = source;",
          "```",
          "",
          "`Bearer <TOKEN>`",
        ].join("\n")}
      />,
    );

    const code = container.querySelectorAll("code");
    expect(code[0]).toHaveTextContent("Vec<String>");
    expect(code[1]).toHaveTextContent("Bearer <TOKEN>");
    expect(code[0]?.textContent).not.toContain("&lt;");
    expect(code[1]?.textContent).not.toContain("&lt;");
  });

  it("restores blockquotes while keeping opening angle brackets literal", () => {
    const { container } = render(
      <TextBlock text={"> Brokered quote\n\nVec<String> and <NotATag>"} />,
    );

    expect(container.querySelector("blockquote")).toHaveTextContent(
      "Brokered quote",
    );
    expect(container).toHaveTextContent("Vec<String>");
    expect(container).toHaveTextContent("<NotATag>");
  });

  it("keeps dangerous tags, attributes, and URLs inert", () => {
    const { container } = render(
      <TextBlock
        text={[
          '<iframe src="https://example.com"></iframe>',
          '<img src=x onerror="alert(1)">',
          '&lt;img src=x onerror="alert(2)"&gt;',
          "[unsafe](javascript:alert(1))",
        ].join("\n")}
      />,
    );

    expect(container.querySelector("iframe")).not.toBeInTheDocument();
    expect(container.querySelector("img")).not.toBeInTheDocument();
    expect(container.querySelector("[onerror]")).not.toBeInTheDocument();
    expect(container).toHaveTextContent(
      '&lt;img src=x onerror="alert(2)"&gt;',
    );
    expect(
      screen.queryByRole("link", { name: "unsafe" }),
    ).not.toBeInTheDocument();
  });

  it("renders linked images without nesting anchors", () => {
    const { container } = render(
      <TextBlock
        text={
          "[![pixel](https://example.com/pixel.png)](https://nyxid.dev/security)"
        }
      />,
    );

    expect(container.querySelectorAll("a")).toHaveLength(1);
    expect(screen.getByRole("link", { name: "pixel" })).toHaveAttribute(
      "href",
      "https://nyxid.dev/security",
    );
    expect(screen.getByTitle("https://example.com/pixel.png").tagName).toBe(
      "SPAN",
    );
  });

  it("renders footnote markers and definitions without dropping content", () => {
    const { container } = render(
      <TextBlock text={"Scoped access[^1]\n\n[^1]: Brokered by NyxID."} />,
    );

    expect(container.querySelector("sup")).toBeInTheDocument();
    expect(container.querySelector("section")).toBeInTheDocument();
    expect(
      screen.getByText("Footnotes", { selector: "section > div" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Brokered by NyxID/)).toBeInTheDocument();
    const footnoteLinks = container.querySelectorAll('a[href^="#"]');
    expect(footnoteLinks.length).toBeGreaterThanOrEqual(2);
    for (const link of footnoteLinks) {
      expect(link).not.toHaveAttribute("target");
    }
  });

  it("glues the caret only to its adjacent paragraph", () => {
    const { container } = render(
      <TextBlock streaming text={"First paragraph.\n\nSecond paragraph."} />,
    );

    const paragraphs = container.querySelectorAll("p");
    const caret = container.querySelector("[data-streaming-caret]");
    expect(paragraphs).toHaveLength(2);
    expect(paragraphs[0]?.nextElementSibling).toBe(paragraphs[1]);
    expect(paragraphs[1]?.nextElementSibling).toBe(caret);
    expect(container.firstElementChild?.className).toContain(
      "[&>p:has(+_[data-streaming-caret])]:inline",
    );
    expect(container.firstElementChild?.className).not.toContain(
      "p:last-of-type",
    );
  });

  it("leaves a streaming caret standalone after a non-paragraph tail", () => {
    const { container } = render(
      <TextBlock
        streaming
        text={[
          "First paragraph.",
          "",
          "Second paragraph.",
          "",
          "| Service | State |",
          "| --- | --- |",
          "| Stripe | Ready |",
        ].join("\n")}
      />,
    );

    const caret = container.querySelector("[data-streaming-caret]");
    const paragraphs = container.querySelectorAll("p");
    expect(caret).toBeInTheDocument();
    expect(paragraphs).toHaveLength(2);
    expect(paragraphs[1]?.nextElementSibling?.querySelector("table"))
      .toBeInTheDocument();
    expect(caret?.previousElementSibling?.tagName).toBe("DIV");
    expect(
      caret?.previousElementSibling?.querySelector("table"),
    ).toBeInTheDocument();
  });
});

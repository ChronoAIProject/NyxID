import { expect, it } from "vitest";
import { render, cleanup } from "@testing-library/react";
import fixtures from "../../../../shared/autolink-punctuation.json";
import { MarkdownContent } from "@/components/assistant/blocks/text-block";
for (const fixture of fixtures)
  it(`bare autolink boundary: ${fixture.text}`, () => {
    const { container } = render(<MarkdownContent text={fixture.text} />);
    expect(
      [...container.querySelectorAll("a")].map((anchor) =>
        anchor.getAttribute("href"),
      ),
    ).toEqual(fixture.hrefs);
    expect(container.textContent).toBe(fixture.text);
    cleanup();
  });
it("explicit Markdown and angle URLs preserve authored punctuation", () => {
  const { container } = render(
    <MarkdownContent text={"[label](https://x.y/z。) <https://x.y/z。>"} />,
  );
  expect(
    [...container.querySelectorAll("a")].map((a) =>
      decodeURI(a.getAttribute("href")!),
    ),
  ).toEqual(["https://x.y/z。", "https://x.y/z。"]);
});

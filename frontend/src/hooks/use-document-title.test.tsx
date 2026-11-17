import { render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { useDocumentTitle } from "./use-document-title";

function Owner({ title }: { readonly title: string | null }) {
  useDocumentTitle(title);
  return null;
}

afterEach(() => {
  document.title = "nyxid";
});

it("sets the title while mounted and restores the application base title", () => {
  const view = render(<Owner title="nyxid - dashboard" />);
  expect(document.title).toBe("nyxid - dashboard");

  view.unmount();

  expect(document.title).toBe("nyxid");
});

it("does not let an older owner's cleanup overwrite a newer owner", () => {
  const first = render(<Owner title="nyxid - dashboard" />);
  expect(document.title).toBe("nyxid - dashboard");
  const second = render(<Owner title="nyxid - docs" />);
  expect(document.title).toBe("nyxid - docs");

  first.unmount();

  expect(document.title).toBe("nyxid - docs");
  second.unmount();
});

it("does not overwrite a title installed outside React", () => {
  const view = render(<Owner title="nyxid - dashboard" />);
  document.title = "external title";

  view.unmount();

  expect(document.title).toBe("external title");
});

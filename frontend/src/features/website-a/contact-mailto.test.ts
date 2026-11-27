// @vitest-environment node
import { describe, expect, it } from "vitest";
import {
  contactEmailHref,
  createContactMailto,
  resolveContactEmail,
} from "./contact-mailto";

const fields = {
  name: "王小明 🟣",
  email: "reply@example.com",
  company: "AI & Co",
  topic: "bundle" as const,
  message: "第一行\nSecond line & questions? #details + 100%",
};

describe("contact mailto encoding", () => {
  it("includes all form details, Unicode and line breaks in only subject/body", () => {
    const uri = createContactMailto(resolveContactEmail(), fields, "zh");
    const url = new URL(uri);
    expect(url.protocol).toBe("mailto:");
    expect(url.pathname).toBe("contact@chrono-ai.fun");
    expect([...url.searchParams.keys()]).toEqual(["subject", "body"]);
    expect(url.searchParams.get("subject")).toBe("NyxID — 定制套餐");
    expect(url.searchParams.get("body")).toBe(
      "姓名: 王小明 🟣\r\n回复邮箱: reply@example.com\r\n公司: AI & Co\r\n主题: 定制套餐\r\n\r\n第一行\r\nSecond line & questions? #details + 100%",
    );
    expect(uri).not.toMatch(/[\r\n]/);
  });

  it("keeps header-shaped and query-shaped user text inside the body", () => {
    const uri = createContactMailto(
      resolveContactEmail(),
      {
        ...fields,
        name: "Alice\r\nBcc: third@example.com",
        company: "Company\nCc: fourth@example.com",
        message:
          "Hello&bcc=evil@example.com\r\nBcc: other@example.com\n%0d%0aSubject: changed",
      },
      "en",
    );
    const params = new URL(uri).searchParams;
    expect([...params.keys()]).toEqual(["subject", "body"]);
    expect(params.get("subject")).toBe("NyxID — Custom bundle");
    expect(params.get("body")).toContain(
      "Name: Alice Bcc: third@example.com\r\n",
    );
    expect(params.get("body")).toContain("Hello&bcc=evil@example.com");
    expect(params.get("body")).toContain("%0d%0aSubject: changed");
    expect(uri).not.toMatch(/[\r\n]/);
  });

  it("omits empty optional company and accepts the original optional topic", () => {
    const params = new URL(
      createContactMailto(
        resolveContactEmail(),
        { ...fields, company: "", topic: "" },
        "en",
      ),
    ).searchParams;
    expect(params.get("subject")).toBe("NyxID — Contact");
    expect(params.get("body")).not.toContain("Company:");
    expect(params.get("body")).toContain("Reply email: reply@example.com");
  });

  it("supports a configured single mailbox and rejects recipient header injection", () => {
    expect(resolveContactEmail(" support+site@example.org ")).toBe(
      "support+site@example.org",
    );
    expect(
      contactEmailHref(resolveContactEmail("support+site@example.org")),
    ).toBe("mailto:support%2Bsite@example.org");
    for (const invalid of [
      "x@example.org\r\nBcc:evil@example.org",
      "x@example.org\n",
      "x@example.org?bcc=evil@example.org",
      "x@example.org,y@example.org",
    ]) {
      expect(() => contactEmailHref(invalid)).toThrow(
        "Invalid contact recipient",
      );
      expect(resolveContactEmail(invalid)).toBe("contact@chrono-ai.fun");
    }
  });

  it("rejects invalid fields before composing a draft", () => {
    expect(() =>
      createContactMailto(
        resolveContactEmail(),
        { ...fields, email: "a@example.org\r\nBcc:evil@example.org" },
        "en",
      ),
    ).toThrow();
    expect(() =>
      createContactMailto(
        resolveContactEmail(),
        { ...fields, message: " " },
        "en",
      ),
    ).toThrow();
  });
});

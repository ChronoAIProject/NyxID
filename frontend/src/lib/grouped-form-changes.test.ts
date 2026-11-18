import { describeChanges, changeList, changeLines, labelValue, recordChanges } from "./grouped-form-changes";
import { expect, it } from "vitest";

it("describes only explicitly allowed fields, with human select labels and On/Off booleans", () => {
  const before = { name: "NyxBot", enabled: false, mode: "ask", token: "private-old-token" };
  const after = { name: "Helper", enabled: true, mode: "auto", token: "private-new-token" };
  const changes = describeChanges(before, after, [{ key: "name", label: "Name" }, { key: "enabled", label: "Enabled" }, { key: "mode", label: "Access mode", format: labelValue({ ask: "Ask first", auto: "Full access" }) }]);
  expect(changes).toEqual([{ label: "Name", before: "NyxBot", after: "Helper" }, { label: "Enabled", before: "Off", after: "On" }, { label: "Access mode", before: "Ask first", after: "Full access" }]);
  expect(JSON.stringify(changes)).not.toContain("token");
});
it("describes grant and membership additions/removals and account reordering", () => {
  expect(describeChanges({ members: ["Ada", "Bob"] }, { members: ["Bob", "Kim"] }, [{ key: "members", label: "Members", list: changeList }])).toEqual([{ label: "Members", added: ["Kim"], removed: ["Ada"] }]);
  expect(describeChanges({ accounts: ["Work", "Personal"] }, { accounts: ["Personal", "Work"] }, [{ key: "accounts", label: "Accounts", list: changeList }])).toEqual([{ label: "Accounts", before: "Work, Personal", after: "Personal, Work" }]);
});
it("handles skill files and shared memory as trimmed line lists", () => {
  expect(describeChanges({ files: "SKILL.md\nold.md" }, { files: " SKILL.md \nnew.md\n" }, [{ key: "files", label: "Files", list: changeLines }])).toEqual([{ label: "Files", added: ["new.md"], removed: ["old.md"] }]);
});
it("describes capability and per-service guest changes with readable labels", () => {
  expect(recordChanges({ shell: false, browser: true }, { shell: true, browser: true }, (key) => key === "shell" ? "Shell commands" : "Secure browser")).toEqual([{ label: "Shell commands", before: "Off", after: "On" }]);
  expect(recordChanges({ notion: "read" }, { notion: "use" }, () => "Notion guest access", labelValue({ read: "Read", use: "Use" }))).toEqual([{ label: "Notion guest access", before: "Read", after: "Use" }]);
});
it("omits unchanged fields and represents cleared text explicitly", () => {
  expect(describeChanges({ note: "Remember", model: "a" }, { note: "", model: "a" }, [{ key: "note", label: "Note" }, { key: "model", label: "Model" }])).toEqual([{ label: "Note", before: "Remember", after: "Empty" }]);
});

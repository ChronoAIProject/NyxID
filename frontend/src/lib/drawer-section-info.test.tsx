import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { translateUi } from "@/i18n/use-ui-translation";
import { GroupedList } from "@/components/ui/grouped-list";
import { AccessModeBadge } from "@/components/assistant/nyxbot-access-controls";
import { DrawerSectionInfoContext, drawerSectionInfo } from "./drawer-section-info";
import { DrawerSearchContext, DrawerSearchTabContext, useDrawerSearchEntries } from "./drawer-search";

afterEach(async () => { cleanup(); await appI18n.changeLanguage("en"); });

it.each(["toString", "__proto__", "constructor", "A custom skill"])("treats arbitrary title %s as unknown", (title) => {
  expect(drawerSectionInfo(title)).toBeUndefined();
});

it("keeps user-supplied skill and note titles verbatim in localized search", async () => {
  await appI18n.changeLanguage("zh-CN");
  const register = vi.fn();
  function UserTitles() {
    useDrawerSearchEntries([{ id: "note", title: "Memory", breadcrumb: "Memory" }, { id: "skill", title: "toString", breadcrumb: "Skills › My skill" }]);
    return null;
  }
  render(<DrawerSearchContext value={{ register, remove: vi.fn(), registerPage: vi.fn(), searching: false }}><DrawerSearchTabContext value="skills"><UserTitles /></DrawerSearchTabContext></DrawerSearchContext>);
  expect(register.mock.calls[0]?.[1]).toMatchObject([{ title: "Memory" }, { title: "toString" }]);
});

it.each(["zh-CN", "zh-TW"])("renders translated headings, help and approval descriptions in %s", async (language) => {
  await appI18n.changeLanguage(language);
  render(<DrawerSectionInfoContext value={true}>
    <GroupedList title={translateUi("Tone and style")} />
    <GroupedList title={translateUi("Call instructions")} />
    <GroupedList title={translateUi("Approval behavior")} />
    <GroupedList title={translateUi("Channel progress reply")} />
    <GroupedList title={translateUi("Preloaded · {{value1}} bytes", { value1: "123" })} />
    <GroupedList title={translateUi("Rule {{value1}}", { value1: 2 })} />
    <AccessModeBadge mode="autopilot" />
  </DrawerSectionInfoContext>);
  for (const title of ["Tone and style", "Call instructions", "Approval behavior", "Channel progress reply", "Preloaded · {{value1}} bytes", "Rule {{value1}}"]) {
    const value1 = title.startsWith("Preloaded") ? "123" : 2;
    const localized = translateUi(title, { value1 });
    const button = screen.getByRole("button", { name: translateUi("About {{value1}}", { value1: localized }) });
    await userEvent.click(button);
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip.textContent).toMatch(/[\u3400-\u9fff]/);
    expect(tooltip).toHaveTextContent(drawerSectionInfo(localized)!);
    await userEvent.keyboard("{Escape}");
  }
  expect(screen.getByLabelText(/Full access|完全访问|完整存取/)).toHaveAttribute("title", expect.stringMatching(/组织连接|組織連線/));
});

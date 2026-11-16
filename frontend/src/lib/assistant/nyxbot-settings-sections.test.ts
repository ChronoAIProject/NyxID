import { expect, it } from "vitest";
import { availableNyxBotSettingsSection, NYXBOT_SETTINGS_SECTIONS, nyxBotSettingsAncestry, nyxBotSettingsParent } from "./nyxbot-settings-sections";

it("derives parents and deep-link ancestry from the shared path vocabulary", () => {
  for (const section of NYXBOT_SETTINGS_SECTIONS) {
    const ancestry = nyxBotSettingsAncestry(section);
    expect(ancestry.at(-1)).toBe(section);
    expect(ancestry).toEqual(section.includes("/") ? [section.split("/")[0], section] : [section]);
    expect(nyxBotSettingsParent(section)).toBe(section.includes("/") ? section.split("/")[0] : undefined);
  }
  expect(nyxBotSettingsAncestry()).toEqual([]);
});

it("falls back to the nearest available ancestor without changing other sections", () => {
  expect(availableNyxBotSettingsSection("channels/adapters", { "channels/adapters": false })).toBe("channels");
  for (const section of NYXBOT_SETTINGS_SECTIONS) {
    expect(availableNyxBotSettingsSection(section, {})).toBe(section);
    if (section !== "channels/adapters") expect(availableNyxBotSettingsSection(section, { "channels/adapters": false })).toBe(section);
  }
});

it("returns the root for an unavailable root section", () => {
  expect(availableNyxBotSettingsSection("notifications", { notifications: false })).toBeUndefined();
});

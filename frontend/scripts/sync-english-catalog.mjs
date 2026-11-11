import fs from "node:fs";

const source = new URL("../src/i18n/locales/en.json", import.meta.url);
const target = new URL("../src/i18n/locales/en-overrides.json", import.meta.url);
const english = JSON.parse(fs.readFileSync(source, "utf8"));
// English phrases already serve as fallback keys; only distinct values need shipping.
const overrides = Object.fromEntries(
  Object.entries(english).filter(([key, value]) => key !== value),
);
fs.writeFileSync(target, JSON.stringify(overrides, null, 2) + "\n");

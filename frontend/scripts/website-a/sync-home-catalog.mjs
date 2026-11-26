// Export only reviewed display fields from an authenticated production catalog snapshot.
// This runs locally with a supplied snapshot; the public website never needs a token.
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "../..");
const directory = path.join(root, "src/features/website-a/nyx-home");
const input = process.argv[2];
if (!input) throw new Error("Pass the path to the reviewed production catalog snapshot.");
const snapshot = JSON.parse(fs.readFileSync(input, "utf8"));
const checkedAt = new Date(snapshot.checked_at);
if (Number.isNaN(checkedAt.valueOf()) || !Array.isArray(snapshot.entries)) {
  throw new Error("The snapshot must contain checked_at and active catalog entries.");
}
const curation = JSON.parse(fs.readFileSync(path.join(directory, "catalog-curation.json"), "utf8"));
const active = new Set(snapshot.entries.map((entry) => entry.slug));
const seen = new Set();
const groups = curation.groups.map((group) => ({
  ...group,
  entries: curation.entries.filter((entry) => entry.group === group.id).flatMap((entry) => {
    const catalogSlugs = entry.slugs.filter((slug) => active.has(slug));
    if (!catalogSlugs.length) return [];
    if (seen.has(entry.id)) throw new Error(`Duplicate display identity: ${entry.id}`);
    seen.add(entry.id);
    const availability = entry.availability ?? "catalog";
    if (!["catalog", "coming_soon"].includes(availability)) throw new Error(`Invalid availability: ${entry.id}`);
    if (entry.icon && !entry.icon.startsWith("/landing/")) {
      const url = new URL(entry.icon);
      if (url.protocol !== "https:" || url.username || url.password) throw new Error("Invalid public icon URL.");
    }
    return [{
      id: entry.id,
      label: entry.label,
      detail: entry.detail ?? null,
      availability,
      icon: entry.icon ?? null,
      catalogSlugs,
    }];
  }),
})).filter((group) => group.entries.length);
const output = { checkedAt: checkedAt.toISOString(), groups };
fs.writeFileSync(path.join(directory, "production-catalog.json"), JSON.stringify(output, null, 2) + "\n");
console.log(`Exported ${seen.size} reviewed integrations from ${snapshot.entries.length} active catalog rows.`);

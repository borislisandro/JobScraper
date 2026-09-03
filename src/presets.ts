// Saved views for the Jobs page. The filters are deliberately view state — they reset to a year and
// no country on every launch, so nothing narrows silently over time — but re-typing "verification"
// and re-picking Portugal every morning is its own kind of friction. A view is that choice, named
// and kept, and applying one is always explicit.
//
// Stored in the settings table under one key, through the get/set commands that already exist: a
// list this short does not need a table, a migration or a command of its own.
import type { JobFilter } from "./api";

export const PRESETS_KEY = "jobs.savedViews";
/// Neither the source ticks nor the triage toggles belong to a view. Source ids go stale as boards
/// come and go, and whether dismissed rows show — or what counts as new — is where you are in
/// today's reading, not what you were looking for.
export type FilterPreset = Omit<JobFilter, "sourceIds" | "includeDismissed" | "firstSeenAfter"> & { name: string };
const MAX_PRESETS = 24;

const asString = (value: unknown) => (typeof value === "string" ? value : "");
const asBool = (value: unknown) => value === true;
const asCodes = (value: unknown) =>
  Array.isArray(value)
    ? [...new Set(value.filter((code): code is string => typeof code === "string" && /^[A-Za-z]{2}$/.test(code)).map(code => code.toUpperCase()))]
    : [];
// A window is a whole number of days or "any time"; anything else came from a hand-edited setting.
const asWindow = (value: unknown) =>
  typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : null;

export const presetFrom = (name: string, filter: JobFilter): FilterPreset => ({
  name: name.trim().slice(0, 60),
  title: filter.title.trim(),
  keyword: filter.keyword.trim(),
  sort: filter.sort,
  savedOnly: filter.savedOnly,
  includeClosed: filter.includeClosed,
  postedWithinDays: filter.postedWithinDays,
  countries: [...filter.countries],
  includeUnknownLocations: filter.includeUnknownLocations,
});
/// Read back defensively: this is a JSON string in a settings row, so it can be anything at all —
/// an older shape, a hand edit, a half-written value. A view that cannot be trusted is dropped
/// rather than allowed to filter the list in some way nobody asked for.
export function parsePresets(raw: string | null | undefined): FilterPreset[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw || "[]");
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const seen = new Set<string>();
  return parsed
    .filter((entry): entry is Record<string, unknown> => !!entry && typeof entry === "object")
    .map(entry => ({
      name: asString(entry.name).trim().slice(0, 60),
      title: asString(entry.title),
      keyword: asString(entry.keyword),
      sort: entry.sort === "posted" ? ("posted" as const) : ("recent" as const),
      savedOnly: asBool(entry.savedOnly),
      includeClosed: asBool(entry.includeClosed),
      postedWithinDays: asWindow(entry.postedWithinDays),
      countries: asCodes(entry.countries),
      includeUnknownLocations: asBool(entry.includeUnknownLocations),
    }))
    .filter(preset => {
      const key = preset.name.toLowerCase();
      if (!preset.name || seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, MAX_PRESETS);
}
/// Saving under a name that already exists replaces it: that is what someone means by saving a view
/// they have just adjusted, and two views with one name cannot be told apart afterwards.
export const withPreset = (presets: FilterPreset[], preset: FilterPreset): FilterPreset[] =>
  [preset, ...presets.filter(existing => existing.name.toLowerCase() !== preset.name.toLowerCase())].slice(0, MAX_PRESETS);
export const withoutPreset = (presets: FilterPreset[], name: string): FilterPreset[] =>
  presets.filter(existing => existing.name.toLowerCase() !== name.toLowerCase());
/// What the view says, in the words the filter bar uses, so a name alone never has to carry it.
export function describePreset(preset: FilterPreset, countryName: (code: string) => string): string {
  const parts: string[] = [];
  if (preset.title) parts.push(`title: ${preset.title}`);
  if (preset.keyword) parts.push(`listing: ${preset.keyword}`);
  if (preset.countries.length) parts.push(preset.countries.map(countryName).join(", "));
  if (preset.includeUnknownLocations) parts.push("including unknown countries");
  if (preset.savedOnly) parts.push("saved and applied only");
  if (preset.includeClosed) parts.push("including closed");
  if (preset.sort === "posted") parts.push("newest posting first");
  parts.push(preset.postedWithinDays ? `posted within ${preset.postedWithinDays} days` : "any posting date");
  return parts.join(" · ");
}

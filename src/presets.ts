// Saved views for the Jobs page, and the filter the page was last left on. A view is a choice
// named and kept, and applying one is always explicit; the last-used filter is restored on its
// own, so closing the app or restarting the computer never costs you "verification" and Portugal.
//
// Both live in the settings table, through the get/set commands that already exist: values this
// small do not need a table, a migration or a command of their own.
import { DEFAULT_POSTED_WINDOW, type JobFilter } from "./api";

export const PRESETS_KEY = "jobs.savedViews";
/// The filter as it was when the page was last used, kept so it is still there on the next launch.
export const ACTIVE_FILTER_KEY = "jobs.activeFilter";
/// Each profile is its own job search, so the filter the Jobs page was left on, the saved views and what
/// counts as new are kept per profile: switching profile opens the other search exactly as it was left.
/// "All profiles" (an empty profile) keeps the original keys.
export const profileKey = (key: string, profile: string) => (profile ? `${key}.${profile}` : key);
/// A profile that has never stored a value of its own starts from the one every profile shared before they
/// were kept apart, so nothing set up earlier is lost. Its first save makes it independent.
export async function readProfileSetting(read: (key: string) => Promise<string | null>, key: string, profile: string) {
  const own = await read(profileKey(key, profile));
  return own ?? (profile ? read(key) : null);
}
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
    .map(entry => ({ name: asString(entry.name).trim().slice(0, 60), ...readView(entry) }))
    .filter(preset => {
      const key = preset.name.toLowerCase();
      if (!preset.name || seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, MAX_PRESETS);
}
/// The filter fields of a stored entry, each one checked: a value that is not what it should be
/// falls back to the default instead of filtering the list in some way nobody asked for.
const readView = (entry: Record<string, unknown>): Omit<FilterPreset, "name"> => ({
  title: asString(entry.title),
  keyword: asString(entry.keyword),
  sort: entry.sort === "posted" ? ("posted" as const) : ("recent" as const),
  savedOnly: asBool(entry.savedOnly),
  includeClosed: asBool(entry.includeClosed),
  postedWithinDays: asWindow(entry.postedWithinDays),
  countries: asCodes(entry.countries),
  includeUnknownLocations: asBool(entry.includeUnknownLocations),
});
export type ActiveFilter = Omit<FilterPreset, "name">;
/// What to store for the last-used filter. Text is kept exactly as typed — trimming would eat a
/// space the next launch should still find.
export const serializeActiveFilter = (filter: ActiveFilter): string =>
  JSON.stringify({
    title: filter.title,
    keyword: filter.keyword,
    sort: filter.sort,
    savedOnly: filter.savedOnly,
    includeClosed: filter.includeClosed,
    postedWithinDays: filter.postedWithinDays,
    countries: filter.countries,
    includeUnknownLocations: filter.includeUnknownLocations,
  });
/// `null` means nothing usable was stored, which is the same as a first launch.
export function parseActiveFilter(raw: string | null | undefined): ActiveFilter | null {
  try {
    const parsed: unknown = JSON.parse(raw || "null");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const entry = parsed as Record<string, unknown>;
    // "Any time" is stored as null; a row with no window at all never chose it.
    return { ...readView(entry), postedWithinDays: "postedWithinDays" in entry ? asWindow(entry.postedWithinDays) : DEFAULT_POSTED_WINDOW };
  } catch {
    return null;
  }
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

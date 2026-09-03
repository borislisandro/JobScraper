import { describe, expect, it } from "vitest";
import { describePreset, parsePresets, presetFrom, withoutPreset, withPreset } from "./presets";
import type { JobFilter } from "./api";

const filter: JobFilter = {
  title: "  verification  ", keyword: "uvm", sort: "posted", savedOnly: false, includeClosed: false,
  postedWithinDays: 30, sourceIds: ["a", "b"], countries: ["PT", "ES"], includeUnknownLocations: false,
  includeDismissed: false, firstSeenAfter: null,
};

describe("saved views", () => {
  it("keeps the filters and never the source selection", () => {
    const view = presetFrom("  Iberia  ", filter);
    expect(view.name).toBe("Iberia");
    expect(view.title).toBe("verification");
    expect(view.countries).toEqual(["PT", "ES"]);
    // Source ids go stale as boards come and go; a view that re-narrowed them would hide jobs
    // nobody chose to hide.
    expect("sourceIds" in view).toBe(false);
  });
  it("survives anything the settings row might hold", () => {
    expect(parsePresets(null)).toEqual([]);
    expect(parsePresets("not json")).toEqual([]);
    expect(parsePresets('{"not":"a list"}')).toEqual([]);
    // A hand-edited row: junk country codes, a nonsense window, a missing name.
    const [only, ...rest] = parsePresets(JSON.stringify([
      { name: "Mixed", countries: ["PT", "nope", 7, "es"], postedWithinDays: -3, sort: "sideways" },
      { title: "no name here" },
    ]));
    expect(rest).toHaveLength(0);
    expect(only.countries).toEqual(["PT", "ES"]);
    expect(only.postedWithinDays).toBeNull();
    expect(only.sort).toBe("recent");
  });
  it("round-trips what was saved", () => {
    const stored = JSON.stringify([presetFrom("Iberia", filter)]);
    expect(parsePresets(stored)[0]).toEqual(presetFrom("Iberia", filter));
  });
  it("replaces a view saved again under the same name", () => {
    const first = withPreset([], presetFrom("Iberia", filter));
    const second = withPreset(first, presetFrom("iberia", { ...filter, title: "analog" }));
    expect(second).toHaveLength(1);
    expect(second[0].title).toBe("analog");
    expect(withoutPreset(second, "IBERIA")).toEqual([]);
  });
  it("says what a view does, so its name never has to", () => {
    const said = describePreset(presetFrom("Iberia", filter), code => ({ PT: "Portugal", ES: "Spain" })[code] ?? code);
    expect(said).toContain("title: verification");
    expect(said).toContain("Portugal, Spain");
    expect(said).toContain("posted within 30 days");
    expect(describePreset(presetFrom("Everything", { ...filter, postedWithinDays: null }), c => c)).toContain("any posting date");
  });
});

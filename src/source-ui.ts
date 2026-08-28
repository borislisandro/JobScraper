import type { Persona, PersonaDetails, WorkerEvent } from "./api";

export const browserCapableAdapters = new Set([
  "playwright", "workday", "eightfold", "icims", "talentbrew-jibe", "phenom",
]);

export function supportsSessionCapture(adapterId: string) {
  return browserCapableAdapters.has(adapterId);
}

export function previewFromEvents(events: WorkerEvent[]) {
  const final = events.at(-1);
  const payload = final?.payload ?? {};
  return {
    jobs: events.filter(event => event.event === "job").map(event => event.payload),
    mode: String(payload.mode ?? "unknown"),
    timing: String(payload.timingMs ?? payload.elapsedMs ?? "n/a"),
    warnings: Array.isArray(payload.warnings) ? payload.warnings.map(String) : [],
    missing: Array.isArray(payload.missingRequiredFields) ? payload.missingRequiredFields.map(String) : [],
    complete: payload.complete === true,
    diagnostics: payload,
  };
}

const strings = (value: string) => {
  try { return JSON.parse(value) as string[]; } catch { return []; }
};

export function personaFormValues(detail?: PersonaDetails): {
  id?: string; name: string; titles: string; skills: string; include: string; exclude: string;
  location: string; workMode: string; seniority: string; salary: string; resume: string;
  includeMode: "any" | "all"; unknown: string; threshold: number;
} {
  const persona: Persona | undefined = detail?.persona;
  return {
    id: persona?.id,
    name: persona?.name ?? "",
    titles: strings(persona?.targetTitlesJson ?? "[]").join(", "),
    skills: detail?.confirmedSkills.join(", ") ?? "",
    include: strings(persona?.includeKeywordsJson ?? "[]").join(", "),
    exclude: strings(persona?.excludeKeywordsJson ?? "[]").join(", "),
    location: persona?.location ?? "",
    workMode: persona?.workMode ?? "",
    seniority: persona?.seniority ?? "",
    salary: persona?.salaryMin?.toString() ?? "",
    resume: persona?.resumeDocumentId ?? "",
    includeMode: persona?.includeKeywordMode ?? "any",
    unknown: persona?.unknownPolicy ?? "pass",
    threshold: persona?.threshold ?? 60,
  };
}

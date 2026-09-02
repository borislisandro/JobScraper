import type { ScrapeAllResult, SourceCheck, WorkerEvent } from "./api";

export const browserCapableAdapters = new Set([
  "playwright", "workday", "eightfold", "icims", "talentbrew-jibe", "phenom",
]);

export function supportsSessionCapture(adapterId: string) {
  return browserCapableAdapters.has(adapterId);
}

// One sentence per worker failure code. Codes, adapter names and raw payloads belong in the
// Diagnostics activity log, not in front of someone who just wants to add a careers page.
const failureText: Record<string, string> = {
  auth_required: "This site requires a login before it will show its jobs.",
  captcha_required: "This site asks for a CAPTCHA before it will show its jobs.",
  robots_denied: "This site asks automated tools not to read its job listings.",
  rate_limited: "This site is asking us to slow down. Try again in a few minutes.",
  browser_incompatible: "Microsoft Edge is needed to read this site and was not found.",
  selector_broken: "This site's job list changed shape and can no longer be read.",
  parse_error: "This website doesn't work with JobScraper.",
  network_error: "Could not reach this site. Check the address.",
  incomplete: "This source is not set up well enough to scrape.",
  cancelled: "Cancelled.",
};
export const describeFailure = (code?: string) =>
  (code && failureText[code]) || "This website doesn't work with JobScraper.";

// Why detection found no jobs. "It doesn't work" is useless when the real problem is a typo in
// the address or a site that refuses automated browsers — those need different action.
const unsupportedText: Record<string, string> = {
  not_found: "That page could not be found. Check the address and try again.",
  blocked: "This site blocks automated tools from reading its job listings.",
  no_browser: "This site needs Microsoft Edge to be read, and Edge was not found.",
  no_listings: "No job listings could be read from this page. Try the page that lists the jobs.",
};
export const describeUnsupported = (reason?: string) =>
  (reason && unsupportedText[reason]) || unsupportedText.no_listings;

export const duration = (milliseconds: unknown) => {
  const value = Number(milliseconds);
  if (!Number.isFinite(value) || value < 0) return "";
  if (value < 1_000) return `${Math.round(value)}ms`;
  if (value < 60_000) return `${(value / 1_000).toFixed(1)}s`;
  return `${Math.floor(value / 60_000)}m ${Math.round((value % 60_000) / 1_000)}s`;
};

export function describeLogEvent(event: WorkerEvent) {
  const payload = event.payload;
  const elapsed = duration(payload.elapsedMs ?? payload.timingMs);
  const timed = (text: string) => elapsed ? `${text} · ${elapsed}` : text;
  const number = (key: string) => Number(payload[key] ?? 0).toLocaleString();
  if (event.event === "started") {
    const action = String(payload.command ?? "run").replace("_source", "").replace("_", " ");
    return `Started ${action} · ${String(payload.adapter ?? "source").toUpperCase()}`;
  }
  if (event.event === "progress" && payload.phase === "fetching") {
    return timed(`Fetching · ${number("requests")} request${Number(payload.requests) === 1 ? "" : "s"}`);
  }
  if (event.event === "progress" && payload.phase === "enriching") {
    const current = Number(payload.current ?? 0), total = Number(payload.total ?? 0);
    const count = total ? ` · ${current.toLocaleString()}/${total.toLocaleString()}` : "";
    return timed(`Descriptions${count} · ${number("requests")} request${Number(payload.requests) === 1 ? "" : "s"}`);
  }
  if (event.event === "progress") {
    const current = Number(payload.current ?? 0), total = Number(payload.total ?? 0);
    const percent = total ? ` (${Math.round(current / total * 100)}%)` : "";
    return timed(`Saving · ${current.toLocaleString()}/${total.toLocaleString()}${percent}`);
  }
  if (event.event === "completed") {
    if (payload.recommendedAdapter) {
      const samples = Array.isArray(payload.sampleJobs) ? payload.sampleJobs.length : 0;
      return timed(`Configured · ${String(payload.recommendedAdapter)} · ${samples} sample${samples === 1 ? "" : "s"}`);
    }
    if (payload.capturedStorageStateBase64) return timed("Session saved");
    if (payload.mode === "enrichment") return timed(`Descriptions done · ${number("persisted")} saved · ${number("failed")} failed · ${number("requests")} requests`);
    if (!("discovered" in payload)) return timed("Done");
    const parts = [`${number("discovered")} found`, `${number("persisted")} saved`];
    if (Number(payload.filtered)) parts.push(`${number("filtered")} filtered`);
    if (Number(payload.skipped)) parts.push(`${number("skipped")} unchanged`);
    parts.push(`${number("requests")} requests`);
    return timed(`Done · ${parts.join(" · ")}`);
  }
  if (event.event === "warning") {
    if (payload.code === "robots_override") return "Robots override active";
    return String(payload.message ?? "Warning");
  }
  if (event.event === "cancelled") return timed("Cancelled");
  if (event.event === "failed") return timed(`Failed · ${String(payload.message ?? describeFailure(payload.code as string | undefined))}`);
  return String(payload.message ?? event.event.replaceAll("_", " "));
}

export function describeOutcome(events: WorkerEvent[], sourceName: string) {
  const final = events.at(-1);
  const payload = final?.payload ?? {};
  if (final?.event === "completed") {
    const found = Number(payload.discovered ?? 0);
    return `${sourceName}: ${found} listing${found === 1 ? "" : "s"} found.`;
  }
  if (final?.event === "cancelled") return `${sourceName}: cancelled.`;
  return `${sourceName}: ${describeFailure(payload.code as string | undefined)}`;
}

export function describeUpdateSummary(result: ScrapeAllResult) {
  const parts = [
    `${result.completedSources} read`,
    `${result.unchangedSources} unchanged`,
  ];
  if (result.failedSources) parts.push(`${result.failedSources} failed`);
  if (result.cancelledSources) parts.push(`${result.cancelledSources} cancelled`);
  return `Updated: ${parts.join(", ")}.`;
}

export function describeCheckSummary(results: SourceCheck[]) {
  const changed = results.filter(result => result.changed);
  const errors = results.filter(result => result.error);
  const uncertain = results.filter(result => !result.changed && !result.conclusive && !result.error);
  const requests = results.reduce((sum, result) => sum + result.requests, 0);
  const parts: string[] = [];
  if (changed.length) {
    parts.push(`Changed: ${changed.map(result => `${result.name}${result.fresh ? ` (${result.fresh} new)` : ""}`).join(", ")}`);
  } else if (!errors.length && !uncertain.length) {
    parts.push(`Nothing new on ${results.length} source${results.length === 1 ? "" : "s"}`);
  }
  if (uncertain.length) parts.push(`Needs an update to confirm: ${uncertain.map(result => result.name).join(", ")}`);
  if (errors.length) parts.push(`Could not check: ${errors.map(result => `${result.name} (${result.error})`).join(", ")}`);
  parts.push(`${requests} request${requests === 1 ? "" : "s"}`);
  return `${parts.join(". ")}.`;
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

/**
 * What a completed probe means for saving. A page that answered but listed nothing is still worth
 * keeping — it saves disabled with the reason on its row, so the address is not lost and can be
 * re-read once the site changes. A page that could not be read at all has nothing to save.
 */
export function probeVerdict(payload: { recommendedAdapter?: unknown; unsupported?: unknown; reason?: unknown; sampleJobs?: unknown }) {
  const adapterId = String(payload.recommendedAdapter || "");
  if (!adapterId || (payload.unsupported && payload.reason !== "no_listings")) {
    return { ok: false as const, error: describeUnsupported(payload.reason as string | undefined) };
  }
  return { ok: true as const, adapterId, found: Array.isArray(payload.sampleJobs) ? payload.sampleJobs.length : 0 };
}

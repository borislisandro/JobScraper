import {readFile,writeFile}from "node:fs/promises";
const tracker=JSON.parse(await readFile(new URL("../docs/company-support-tracker.json",import.meta.url),"utf8"));
// The backlog is filtered to one engineering discipline (ASIC, digital design and verification,
// embedded software, mixed-signal, hardware). Out-of-scope rows keep their sequence number and are
// listed at the end with the reason, so the filter is auditable and reversible rather than a
// deletion — and so the counts above the queue describe the work that is actually planned.
const inScope=tracker.companies.filter(company=>company.inScope!==false);
const outOfScope=tracker.companies.filter(company=>company.inScope===false);
const counts=Object.entries(Object.groupBy(inScope,company=>company.status)).map(([status,companies])=>`${companies.length} ${status}`).join(" · ");
const escape=value=>String(value??"—").replaceAll("|","\\|").replaceAll("\n"," ");
// The focus list is the only ordering that matters day to day: everything else is a backlog, and a
// backlog of 187 is not a plan. These are worked first regardless of their priority group.
const focus=tracker.companies.filter(company=>company.focus);
const focusRows=focus.map(company=>`| ${company.sequence} | ${escape(company.name)} | ${escape(company.category)} | **${company.status}** | ${escape(company.focusReason)} |`).join("\n");
const skipped=outOfScope.map(company=>`| ${company.sequence} | ${escape(company.name)} | ${escape(company.category)} | ${escape(company.outOfScopeReason)} |`).join("\n");
// Each priority group's scope line and row range are read from the rows themselves, so filtering
// or reordering the backlog cannot leave a stale summary sitting above the queue.
const priorities=Object.entries(Object.groupBy(tracker.companies,company=>company.priority)).sort().map(([priority,group])=>{
 const kept=group.filter(company=>company.inScope!==false),sequences=kept.map(company=>company.sequence);
 const scope=[...new Set(kept.map(company=>company.category))].join(", ")||"—";
 return `| ${priority} | ${escape(scope)} | ${kept.length} of ${group.length} | ${kept.length?`Process rows ${Math.min(...sequences)}–${Math.max(...sequences)} in order`:"Nothing in scope"} |`}).join("\n");
const rows=inScope.map(company=>`| ${company.sequence} | ${company.priority} | ${escape(company.name)} | ${escape(company.category)} | **${company.status}** | ${escape(company.adapterId)} | ${company.evidence.map(path=>`[proof](${path.replace(/^docs\//,"")})`).join(" ")||"—"} | ${escape(company.nextAction)}${company.notes?` ${escape(company.notes)}`:""} |`).join("\n");
const plan=`# Engineering company expansion plan

Updated: ${tracker.updatedAt}. **${inScope.length} in-scope employers** of ${tracker.companies.length} candidates. ${counts}.

## Extreme priority: the focus list

${focus.length} employers, worked before anything else. These are the ones that hire digital design and
verification engineers — RTL, testbenches, silicon bring-up — and that the catalog does not already
cover. The catalog already ships Arm, AMD, NVIDIA, Intel, Qualcomm, Broadcom, Apple, MediaTek,
Marvell, Infineon, STMicroelectronics, NXP, Texas Instruments, Analog Devices, Microchip, Lattice,
Silicon Labs, GlobalFoundries, Micron, SiFive, Astera Labs, Tenstorrent, Lightmatter, SambaNova,
Synopsys and Cadence, so none of them appear below.

| Order | Company | Category | Status | Why it is on this list |
| ---: | --- | --- | --- | --- |
${focusRows}

## Objective and working order

${tracker.objective}

This is the backlog from the requested missing-company list, filtered to employers that do ASIC, digital design, digital verification, embedded software, mixed-signal or hardware engineering. A row is a candidate employer or brand, not a promise that its board is currently readable. The ${outOfScope.length} rows outside that discipline are kept at the bottom of this file rather than deleted. The machine-readable source of status is [company-support-tracker.json](company-support-tracker.json). Regenerate this view with \`node scripts/update-company-plan.mjs\`.

| Priority | Scope | In scope | Completion condition |
| --- | --- | ---: | --- |
${priorities}

Work on one company at a time within each batch. Batches run in parallel as subagents following [company-onboarding-runbook.md](company-onboarding-runbook.md); each owns only its own candidate and proof files. Shared adapters, the catalog and the tracker have one integration owner, who reviews evidence and admits validated entries with \`node scripts/admit-company.mjs\`. Later phases stay queued until the current one is assessed.

Reuse a proven platform adapter when its live response matches; otherwise implement the smallest company adapter at the existing guarded request seam. Shared-family work is justified when the observed contract serves multiple queued employers. Do not guess tenants, tokens, endpoints, or ownership from a similar company name.

## Status definitions

- **queued**: not examined in this expansion; only the candidate name is known.
- **discovering**: identifying the official board, regional scope, robots policy, and actual API or HTML contract.
- **implementing**: configuration or adapter changes are in progress; the company is not supported yet.
- **validating**: offline regression checks and live worker proof are in progress.
- **supported**: the catalog entry is present and all acceptance gates below have saved evidence.
- **blocked**: a concrete policy, access, missing data, or technical obstacle remains. Record the observed failure and next step, then continue to the next queued row. Revisit after the current priority group.
- **covered_by_parent**: an existing parent board demonstrably includes the brand's jobs; link its source and proof. A corporate acquisition alone is not coverage evidence.

## Per-company implementation and acceptance

1. **Discover and identify.** Follow the employer's official careers page to its actual board. Record both the official page and selected board, relevant regions, and any separate subsidiary boards. Confirm company identity against employer-owned content.
2. **Check access.** Robots enforcement is off by choice for this tool, matching what \`install_starter_pack\` already seeds, so a \`Disallow\` does not stop a read. Private-network protection, per-origin pacing and the URL guard stay on, and no authentication bypass or guessed private endpoint is acceptable. Record what the host's robots policy says anyway: it usually names the endpoint that serves the vacancies.
3. **Configure or implement.** Tune only settings justified by observed responses: page size, paging limits, detail strategy, and pacing. Add a custom adapter when required, including registry, detection/editor support, normalization and packaging integration as applicable.
4. **Add regression proof.** Exercise the observed listing/detail shape, pagination termination, stable identity, malformed/partial reads, and any new branch that could falsely mark a board complete. Keep company-specific fixtures compact and free of unnecessary job content.
5. **Full cold read.** Run the real worker's \`scrape_source\` protocol over the live board with no known hashes or title filters. Require non-empty normalized jobs, valid HTTP(S) application links, unique identities and listing hashes, and \`complete: true\`. Reconcile a published exact total with discovered rows. A first-page preview is insufficient.
6. **Description and incremental proof.** Verify real description content, fetching deferred details for representative first/middle/last jobs where available. Run a warm change check using the cold read's hashes and record freshness/count behavior. A moving board is investigated rather than mislabeled as an adapter failure.
7. **Publish locally.** Only after live proof succeeds, add the ready-to-use catalog configuration with a stable ID and verification reference. Leave starter membership unchanged. Prove catalog request construction and run affected sidecar, Rust and frontend gates.
8. **Update status.** Save the compact proof report (source/config, timestamp, worker code hashes, requests, counts, completion, samples, details, and warm check), set the row to supported, regenerate this plan, then proceed to the next sequence number.

## Safety and completion rules

Partial, failed, capped, malformed, or empty reads never authorize closing stored jobs. Preserve current per-origin pacing, cancellation, SSRF defenses, and starter enable/delete choices. A browser-rendered fallback must preserve the same access policy and must prove its pagination before it is accepted.

Do not add 269 disabled guesses to the app. Pending and blocked work stays in this tracker; the picker contains only proven configurations. A blocked company remains unfinished until access changes or a verified public alternative is implemented. The overall objective is complete only when every row is supported or demonstrably covered by a parent board; unresolved blocked rows must remain visible.

Existing 133 supported employers retain their earlier verification level. The stronger full-read gate applies to new entries added by this expansion. Live proof certifies the recorded run, not permanent availability of a third-party board.

## Reproducing acceptance

Before catalog admission, save the candidate as a worker source JSON, then run \`node scripts/verify-company-catalog.mjs --full --candidate path/to/source.json --report docs/company-proofs/company.json\`. The command exits unsuccessfully on incomplete traversal, duplicate identities, missing descriptions, count mismatch or unstable warm results. Inspect the saved samples and warnings before admission.

After admission, rerun by exact catalog name: \`node scripts/verify-company-catalog.mjs --full "ASML" --report docs/company-proofs/asml.json\`. Without \`--full\`, the command is only a first-page preview and does not satisfy acceptance. The catalog contract test requires supported expansion rows to have full evidence matching the shipped configuration.

## Company-by-company queue

| Order | Priority | Company | Category | Status | Adapter | Evidence | Next step / notes |
| ---: | --- | --- | --- | --- | --- | --- | --- |
${rows}

## Out of scope

Recorded, not planned. These employers sit outside ASIC, digital design and verification, embedded software, mixed-signal and hardware work. They keep their original sequence number, so lifting the filter renumbers nothing.

| Order | Company | Category | Why it is out of scope |
| ---: | --- | --- | --- |
${skipped}
`;
await writeFile(new URL("../docs/company-expansion-plan.md",import.meta.url),plan);
console.log(counts);

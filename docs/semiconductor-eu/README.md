# Semiconductor EU expansion — results (2026-11-06)

Employers added for design-verification, DFT, RTL, ASIC and FPGA roles in PT, ES, FR, NL, IE, AT, DE, UK and CH. Every catalog entry below has a passing full proof in `docs/company-proofs/` (complete traversal, unique identities and hashes equal to the job count, warm re-check finding nothing new).

"Role titles" counts job titles matching DV, DFT, RTL, ASIC, FPGA, SoC, UVM, verification, digital/logic/physical/IC design and similar (`scripts/relevance-check.mjs`); "in target" is the subset whose location is in the nine countries. Counts are a snapshot and will drift.

## Admitted

| Company | Adapter | Jobs | Role titles | In target | Default list |
| --- | --- | ---: | ---: | ---: | :---: |
| Semidynamics | json | 38 | 13 | 12 | yes |
| Bosch | bosch | 779 | 7 | 7 | no |
| Arago | ashby | 15 | 6 | 6 | yes |
| Codasip | bamboohr | 9 | 6 | 4 | yes |
| Fractile | ashby | 25 | 4 | 4 | yes |
| Raspberry Pi | workable | 7 | 4 | 4 | yes |
| Teledyne | workday | 768 | 13 | 4 | no |
| Universal Quantum | workable | 37 | 4 | 4 | no |
| Baya Systems | greenhouse | 23 | 9 | 3 | yes |
| Olix | ashby | 22 | 3 | 3 | yes |
| SEMRON | personio | 11 | 3 | 3 | no |
| Dolphin Semiconductor | json | 4 | 2 | 2 | yes |
| IonQ | greenhouse | 120 | 3 | 2 | no |
| Quantum Motion | personio | 19 | 2 | 2 | no |
| Riverlane | greenhouse | 37 | 2 | 2 | yes |
| CERN | cern | 56 | 1 | 1 | no |
| Pragmatic Semiconductor | json | 15 | 1 | 1 | yes |
| Prophesee | json | 9 | 1 | 1 | yes |
| Axiomise | personio | 4 | 4 | 0 | yes |
| Elmos Semiconductor | json | 19 | 0 | 0 | no |
| Kalray | json | 15 | 2 | 0 | no |
| Philips | workday | 810 | 5 | 0 | no |

Elmos shows 0 because its digital IC postings are titled "Development Engineer (m/f/d) Digital" / "Entwicklungsingenieur Digital", which the title pattern does not match; they are real digital-design roles. Philips shows 0 in the target countries and is kept only for catalog breadth.

## Blocked or deferred

| Company | Why |
| --- | --- |
| Rohde & Schwarz | 2026-11-06: the Avature board (jobs.rohde-schwarz.com) lists 168 vacancies, none in Germany and none a chip-design role: US 58, India 36, Japan 14, Singapore 14, Romania 7, Spain 5. German R&S roles are not on a board the app can read, so nothing relevant to DV/DFT/ASIC in the target countries. |
| ams OSRAM | 2026-11-06: careers.ams-osram.com is WordPress and jobs.ams-osram.com is a Next.js app that streams its job list client-side; the server HTML carries only three featured vacancies, and no JSON route was found. |
| Paul Scherrer Institut | 2026-11-06: psi.ch/en/jobs answers 404 to a plain request and no Refline job feed URL was found; ASIC-designer roles exist per research but no readable board. |
| Dream Chip Technologies | 2026-11-06: the career page is a Framer site; vacancy pages are static but no list endpoint or sitemap of postings was found (only searchIndex JSON). |
| imec | 2026-11-06: Drupal front end; drupal.imec-int.com/api/jobs returns only the filter facets (types, regions, employers), not vacancies. The search call that fills the list was not found without a browser. |
| Aion Silicon | 2026-11-06: jobs sit on a Deel job board (jobs.deel.com/job-boards/careersaionsilicon) that renders client-side; no __NEXT_DATA__ or JSON in the server HTML. About 5 jobs. |
| onsemi | 2026-11-06: Limerick/Cork DV demand is strong, but the careers site answers 403 to automated requests and the ATS is unknown. |
| TDK-Micronas | 2026-11-06: careers site answers 403 to automated requests; ATS not identified. |
| Melexis | 2026-11-06: careers site answers 403 to automated requests; ATS not identified. |
| MED-EL | 2026-11-06: ATS not identified from a plain request. |
| Sensirion | 2026-11-06: ATS not identified from a plain request. |
| EM Microelectronic | 2026-11-06: ATS not identified from a plain request. |
| VSORA | 2026-11-06: openings are listed on the company site with no ATS behind them. |
| Barcelona Supercomputing Center | 2026-11-06: in-house job pages, no ATS API found. |
| USound | 2026-11-06: jobs run on an Odoo website; no JSON route identified. |
| ADVANS / ELSYS Lisbon | 2026-11-06: Lisbon DV roles exist but no ATS was discoverable. No Portuguese chip employer is on a readable job board; Portugal coverage comes from Synopsys (Moreira), Monolithic Power (Porto), Infineon and the existing starters. |

The Tier 4 entries (onsemi, TDK-Micronas, Melexis, MED-EL, Sensirion, EM Microelectronic, VSORA, BSC, USound) carry the reasons found during research; they were not probed again when recorded.

## Tips for filter setup

- The title and notification filters match what you type. "DV" also matches the UK security clearance "DV Cleared" (seen at Lockheed Martin and Arista), so prefer "design verification" or "verification" over the bare abbreviation.
- Location text such as "Manchester, NH" or "Cambridge, MA" matches a UK city name. Filter by country rather than by city.

## Regenerating

```
node docs/semiconductor-eu/make-candidates.mjs ... make-candidates-6.mjs   # candidate sources
docs/semiconductor-eu/prove.sh <id>                                        # full proof
node scripts/relevance-check.mjs docs/company-candidates/<id>.json --write # role counts
docs/semiconductor-eu/admit.sh <id> "<Name>" <CC> <tags> <careersUrl>      # catalog + tracker
node docs/semiconductor-eu/promote.mjs <version>                           # default list + version bump
node docs/semiconductor-eu/starter-fixture.mjs                             # fixture without a Windows build
```

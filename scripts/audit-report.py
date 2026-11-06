"""Builds docs/scrape-audit/scraper-vs-website.xlsx from the two audit runs.
Inputs (written by scripts/audit-scrape.mjs and scripts/audit-site.mjs): docs/scrape-audit/corpus/*.json and docs/scrape-audit/site/*.json.
Numbers are data; the status of each company is an Excel formula over them, with the thresholds in the Method sheet."""
import json, re, os, sys
from openpyxl import Workbook
from openpyxl.styles import Font, PatternFill, Alignment, Border, Side
from openpyxl.utils import get_column_letter
from openpyxl.formatting.rule import CellIsRule, FormulaRule
from openpyxl.worksheet.datavalidation import DataValidation

ROOT = os.path.join(os.path.dirname(__file__), '..')
slug = lambda n: re.sub(r'^-|-$', '', re.sub(r'[^a-z0-9]+', '-', n.lower()))
catalog = [c for c in json.load(open(f'{ROOT}/sidecar/company-catalog.json'))['companies'] if c['kind'] != 'reference']
starter = {c['name'] for c in catalog if c.get('starter')}
TERMS = ['engineer', 'verification']
norm = lambda t: re.sub(r'[^a-z0-9]+', ' ', t.lower()).strip()
word = lambda term, title: re.search(r'\b' + term + r's?\b', title, re.I) is not None

NOTES = {
 'NVIDIA': 'Workday\'s own search page stops at 2,000 results; the scraper splits the board by job family and reads all of it, so it is expected to exceed the page.',
 'Qualcomm': 'Eightfold rate-limited the scraper mid-read ("rate_limited") on every attempt, including one run alone. The app would stop at the same point.',
 'Lockheed Martin': 'Eightfold rate-limited the scraper mid-read on every attempt. Roughly one fifth of the board was read.',
 'Northrop Grumman': 'Eightfold refused the scraper partway ("auth_required" / rate limit) on every attempt; most of the board was read.',
 'Doctolib': 'The Greenhouse board address returns 404, so the board has moved or been closed; the catalog entry needs a new address.',
 'Postman': 'The Greenhouse board address returns 404, so the board has moved or been closed; the catalog entry needs a new address.',
 'Confluent': 'The Ashby board publishes no listings at all; the careers page showed none either.',
 'Pipedrive': 'The Lever board publishes no listings at all; the careers page showed none either.',
 'Marvell': 'First two scrapes failed (HTTP 502, then a timeout); a third succeeded with 348 jobs, matching the page. Treated as a transient server fault.',
 'Elastic': 'The Greenhouse API lists far more vacancies than the company\'s own careers page displayed; the page may filter or paginate in a way the browser did not follow.',
 'Oracle': 'The scraper read 2,833 of a published 2,838; the careers page lists jobs without links the browser could count, so only the board total was compared.',
 'Sierra Space': 'The page states 180 openings, the scraper stored 200; unresolved difference of about 10%.',
 'ASML': 'Scraper uses the new bearer-token search; the careers page draws its list with scripts the browser did not expose as links.',
}
rows = []
for c in catalog:
    s = slug(c['name'])
    sp, wp = f'{ROOT}/docs/scrape-audit/corpus/{s}.json', f'{ROOT}/docs/scrape-audit/site/{s}.json'
    scr = json.load(open(sp)) if os.path.exists(sp) else None
    web = json.load(open(wp)) if os.path.exists(wp) else None
    r = {'name': c['name'], 'adapter': c['adapterId'], 'country': c['country'], 'starter': 'yes' if c['name'] in starter else 'no', 'url': c['baseUrl']}
    if scr:
        titles = [j['title'] for j in scr['jobs']]
        r.update(scr_ok='yes' if scr['ok'] else 'no', scr_complete='yes' if scr['complete'] else 'no', scr_jobs=len(titles), scr_err=(scr.get('error') or '')[:140],
                 scr_eng=sum(word('engineer', t) for t in titles), scr_ver=sum(word('verification', t) for t in titles), scr_secs=round(scr['durationMs'] / 1000))
        sset = {norm(t) for t in titles}
    else:
        sset = set(); r.update(scr_ok='not run')
    if web:
        base = web.get('baseline') or {}
        links = base.get('links') or 0
        MIN = 2 if c['adapterId'] in ('greenhouse', 'ashby', 'lever', 'workable', 'personio', 'bamboohr') else 5
        stated = base.get('reported') or None
        r.update(web_url=(web.get('triedUrls') or [web['url']])[-1], web_err=(web.get('error') or '')[:120],
                 web_blocked='yes' if (web.get('blockedHint') and links < 5) else 'no', web_reported=stated, web_links=links)
        # Board total. Exact when the page's own list was loaded to (about) the size it states, or when a platform whose page states a reliable
        # "N jobs" figure says so and nothing contradicts it; otherwise whatever was loaded is only a lower bound.
        RELIABLE = {'workday', 'eightfold', 'oracle', 'greenhouse', 'ashby', 'lever'}
        if links >= MIN and (stated is None or links >= 0.9 * stated):
            r['web_total'], r['total_kind'], r['total_src'] = links, 'exact' if stated is not None else 'at least', 'page lists %d rows' % links
        elif stated is not None and stated >= links and c['adapterId'] in RELIABLE and stated > 0:
            r['web_total'], r['total_kind'], r['total_src'] = stated, 'exact', 'page states %d' % stated
        elif links >= MIN:
            r['web_total'], r['total_kind'], r['total_src'] = links, 'at least', 'page lists %d rows, states %s (not trusted)' % (links, stated)
        else:
            r['web_total'], r['total_kind'], r['total_src'] = None, '', ''
        for t in TERMS:
            tt = web['terms'].get(t, {}); k = t[:3]
            n = kind = None
            if tt.get('searchBox') and tt.get('resultLinks'):
                n = sum(word(t, x) for x in (tt.get('matchingTitles') or [])); rep = tt.get('reported')
                kind = 'exact' if (rep is not None and tt['resultLinks'] >= 0.9 * rep) else 'at least'
            elif links >= MIN:
                n = sum(word(t, x) for x in (base.get('titles') or [])); kind = 'at least'
            r['web_' + k], r['kind_' + k] = n, kind or ''
            r['webrep_' + k] = tt.get('reported')
        wt = [norm(x) for x in (base.get('titles') or [])]
        def seen(t):
            if t in sset: return True
            return len(t) >= 8 and any((t in x or x in t) for x in slist)
        slist = list(sset) if len(sset) <= 8000 else []
        r['overlap'] = (sum(seen(t) for t in wt) / len(wt)) if len(wt) >= 5 and sset else None
    rows.append(r)

wb = Workbook()
F = 'Arial'
hdr_font = Font(name=F, bold=True, color='FFFFFF', size=10); hdr_fill = PatternFill('solid', fgColor='1F3864')
body = Font(name=F, size=10); bold = Font(name=F, size=10, bold=True); inp = Font(name=F, size=10, color='0000FF', bold=True)
thin = Side(style='thin', color='D9D9D9'); box = Border(top=thin, bottom=thin, left=thin, right=thin)

# ---------------- Method ----------------
m = wb.active; m.title = 'Method'
m.column_dimensions['A'].width = 34; m.column_dimensions['B'].width = 14; m.column_dimensions['C'].width = 100
m['A1'] = 'Scraper vs website audit'; m['A1'].font = Font(name=F, size=14, bold=True)
m['A2'] = 'Every company the app currently supports (the catalog, reference bookmarks excluded). Blue cells are the thresholds the Status formulas use.'; m['A2'].font = body
lines = [
 ('Tolerance (relative)', 0.10, 'A scraper count matches the website when it is within this share of the website count (boards change between the two reads).'),
 ('Tolerance (absolute jobs)', 5, '...or within this many jobs, so tiny boards are not judged on a one-job difference.'),
 ('Partial threshold', 0.50, 'Below "match" but at least this share of the website count counts as partially working; below it, not working.'),
 ('Title overlap needed', 0.80, 'Share of the titles the website page listed that the scraper also stored, for the title-overlap check.'),
]
m['A4'] = 'Thresholds'; m['A4'].font = bold
for i, (a, v, note) in enumerate(lines):
    r = 5 + i; m[f'A{r}'] = a; m[f'B{r}'] = v; m[f'C{r}'] = note
    m[f'A{r}'].font = body; m[f'B{r}'].font = inp; m[f'C{r}'].font = body
m['B5'].number_format = '0%'; m['B7'].number_format = '0%'; m['B8'].number_format = '0%'
text = [
 ('How each number was obtained', None),
 ('Scraper side', 'scripts/audit-scrape.mjs runs the app\'s real worker over each company\'s catalog configuration (full scrape, descriptions deferred) and keeps every title, place and link it would store. "engineer" and "verification" are counted over those titles.'),
 ('Website side', 'scripts/audit-site.mjs opens the company\'s real careers page in Chrome, loads the whole list (load-more, next page, scrolling, iframes), types the term in the page\'s own search box and counts the vacancy links whose title contains it. Nothing on this side goes through the scraper.'),
 ('Same question both sides', 'Both sides count job TITLES containing the term. A site search also matches descriptions, so its own "N results" figure is higher; it is shown separately (Site reported) and is not used for judging.'),
 ('Four checks', 'Board total (scraper jobs vs the count the careers page states), "engineer" count, "verification" count, and title overlap (share of titles the page listed that the scraper also has). A check only counts when the website gave a usable number.'),
 ('Status', 'Working = every available check matches, at least one of them against an exact website number, and the scraper read the whole board. Likely working = every available check matches but the website could only be partly read, so none was exact. Partially working = some check matches, or the scraper is at least the partial threshold of the website, or the read was cut short. Not working = scraper failed, returned nothing, or is far below the website. Cannot verify = the website could not be read in an automated browser (blocked, no list, error) so there is nothing to compare with; the scraper numbers are still shown.'),
 ('Exact vs at least', 'A website number is "exact" only when the page was loaded to the size it states for itself (or, for Workday, Eightfold, Oracle, Greenhouse, Ashby and Lever, when the page states a board total and nothing contradicts it). Otherwise it is "at least": what a browser managed to load, which is a lower bound. An exact number must match the scraper within the tolerances; an "at least" number only requires the scraper to have found at least that many. A scraper that finds MORE than a partly loaded page shows is not penalised.'),
 ('Garbage guards', 'Some pages print numbers that are not a board total (a footer count, a filter badge). A stated total is ignored when a platform is not on the reliable list or when the page itself listed more rows than it claimed; the raw stated figure stays in its own column.'),
 ('Limits', 'A browser-driven count is only as good as the page: a site that loads results lazily or caps its list can understate the website number, and a page that hides vacancies behind a login or bot check cannot be read at all. Where the website number is lower than the scraper and the scraper\'s own board total agrees with the scraper count, treat the site read as the weaker side. The two reads happened minutes apart, so a +/- 1-2 difference is board movement.'),
]
r = 11
for a, b in text:
    m[f'A{r}'] = a; m[f'A{r}'].font = bold
    if b: m[f'B{r}'] = None; m[f'C{r}'] = b; m[f'C{r}'].font = body; m[f'C{r}'].alignment = Alignment(wrap_text=True, vertical='top'); m.row_dimensions[r].height = 62
    r += 1

# ---------------- Companies ----------------
ws = wb.create_sheet('Companies')
cols = [('Company', 24), ('Default list', 8), ('Adapter', 14), ('Country', 8),
        ('Scraper ran', 9), ('Scraper complete', 10), ('Scraper jobs', 9), ('Scraper "engineer"', 10), ('Scraper "verification"', 11), ('Scrape secs', 8),
        ('Site total', 11), ('Site total is', 9), ('Site "engineer"', 10), ('Site "engineer" is', 9), ('Site "verification"', 11), ('Site "verification" is', 9), ('Site list loaded (links)', 10), ('Site read', 14),
        ('Total: scraper/site', 10), ('Engineer: scraper/site', 10), ('Verification: scraper/site', 11), ('Title overlap', 9),
        ('Checks matching', 9), ('Checks available', 9), ('Exact checks available', 9), ('Status', 18),
        ('Notes', 60), ('Scraper error', 38), ('Site error / note', 38), ('Site "engineer" reported (incl. descriptions)', 12), ('Site "verification" reported (incl. descriptions)', 12), ('How the site total was read', 30), ('Site stated total (raw, not always trustworthy)', 12), ('Careers page used', 44)]
for i, (h, w) in enumerate(cols, 1):
    c = ws.cell(row=1, column=i, value=h); c.font = hdr_font; c.fill = hdr_fill; c.alignment = Alignment(wrap_text=True, vertical='center', horizontal='center'); c.border = box
    ws.column_dimensions[get_column_letter(i)].width = w
ws.row_dimensions[1].height = 54
L = {h: get_column_letter(i) for i, (h, _) in enumerate(cols, 1)}
rows.sort(key=lambda r: r['name'].lower())
for n, r in enumerate(rows, 2):
    ran = r.get('scr_ok') != 'not run'
    web_state = 'not run' if 'web_links' not in r else ('error' if r['web_err'] else ('blocked' if r['web_blocked'] == 'yes' else ('list read' if r['web_links'] >= (2 if r['adapter'] in ('greenhouse', 'ashby', 'lever', 'workable', 'personio', 'bamboohr') else 5) else ('little read' if r['web_links'] else 'nothing read'))))
    note = r.get('web_err') or ('Page looks like a bot check or block.' if r.get('web_blocked') == 'yes' else '')
    vals = [r['name'], r['starter'], r['adapter'], r['country'],
            r.get('scr_ok'), r.get('scr_complete'), r.get('scr_jobs'), r.get('scr_eng'), r.get('scr_ver'), r.get('scr_secs'),
            r.get('web_total'), r.get('total_kind'), r.get('web_eng'), r.get('kind_eng'), r.get('web_ver'), r.get('kind_ver'), r.get('web_links'), web_state]
    for i, v in enumerate(vals, 1):
        c = ws.cell(row=n, column=i, value=v); c.font = body; c.border = box
    a = lambda h: f'{L[h]}{n}'
    ok = f'{a("Scraper ran")}="yes"'
    def ratio(sc, wb_):
        return f'=IF(AND({ok},ISNUMBER({a(wb_)}),{a(wb_)}>0),{a(sc)}/{a(wb_)},IF(AND({ok},ISNUMBER({a(wb_)}),{a(wb_)}=0,{a(sc)}=0),1,""))'
    ws[a('Total: scraper/site')] = f'=IF(AND({ok},ISNUMBER({a("Site total")}),{a("Site total")}>0,{a("Site read")}<>"error",{a("Site read")}<>"blocked"),{a("Scraper jobs")}/{a("Site total")},"")'
    ws[a('Engineer: scraper/site')] = ratio('Scraper "engineer"', 'Site "engineer"')
    ws[a('Verification: scraper/site')] = ratio('Scraper "verification"', 'Site "verification"')
    ov = r.get('overlap'); ws[a('Title overlap')] = ov if ov is not None else ''
    # a check "matches" when |scraper - site| <= max(abs tolerance, rel tolerance * site); ratio >= 1-tol is not enough on its own because the scraper can be higher.
    def match(ratio_col, sc, wb_, kind):
        diff = f'ABS({a(sc)}-{a(wb_)})<=MAX(Method!$B$6,Method!$B$5*{a(wb_)})'
        floor = f'{a(sc)}>={a(wb_)}-MAX(Method!$B$6,Method!$B$5*{a(wb_)})'
        return f'IF({a(ratio_col)}="",0,IF(IF({a(kind)}="exact",{diff},{floor}),1,0))'
    SE, SV, WE, WV = 'Scraper "engineer"', 'Scraper "verification"', 'Site "engineer"', 'Site "verification"'
    ws[a('Checks matching')] = ('=' + match("Total: scraper/site", "Scraper jobs", "Site total", "Site total is") + '+' + match("Engineer: scraper/site", SE, WE, 'Site "engineer" is')
                                + '+' + match("Verification: scraper/site", SV, WV, 'Site "verification" is') + f'+IF({a("Title overlap")}="",0,IF({a("Title overlap")}>=Method!$B$8,1,0))')
    ws[a('Exact checks available')] = (f'=IF(AND({a("Total: scraper/site")}<>"",{a("Site total is")}="exact"),1,0)+IF(AND({a("Engineer: scraper/site")}<>"",{a("Site \"engineer\" is")}="exact"),1,0)'
                                       f'+IF(AND({a("Verification: scraper/site")}<>"",{a("Site \"verification\" is")}="exact"),1,0)')
    ws[a('Checks available')] = f'=IF({a("Total: scraper/site")}="",0,1)+IF({a("Engineer: scraper/site")}="",0,1)+IF({a("Verification: scraper/site")}="",0,1)+IF({a("Title overlap")}="",0,1)'
    # smallest comparable ratio, used for the partial/not-working boundary
    minr = f'MIN(IF({a("Total: scraper/site")}="",9,{a("Total: scraper/site")}),IF({a("Engineer: scraper/site")}="",9,{a("Engineer: scraper/site")}),IF({a("Verification: scraper/site")}="",9,{a("Verification: scraper/site")}),IF({a("Title overlap")}="",9,{a("Title overlap")}))'
    ws[a('Status')] = (f'=IF({a("Scraper ran")}<>"yes","Not working",IF({a("Scraper jobs")}=0,"Not working",'
                       f'IF(AND({a("Site total is")}="exact",{a("Total: scraper/site")}<>"",{a("Total: scraper/site")}<Method!$B$7),"Not working",'
                       f'IF({a("Checks available")}=0,"Cannot verify",'
                       f'IF(AND({a("Checks matching")}={a("Checks available")},{a("Scraper complete")}="yes"),IF({a("Exact checks available")}>=1,"Working","Likely working"),'
                       f'IF(OR({a("Checks matching")}>=1,{minr}>=Method!$B$7),"Partially working","Not working"))))))')
    ws[a('Notes')] = NOTES.get(r['name'], '')
    ws[a('Scraper error')] = r.get('scr_err', '') or ''
    ws[a('Site error / note')] = note
    ws[a('How the site total was read')] = r.get('total_src', ''); ws[a('Site stated total (raw, not always trustworthy)')] = r.get('web_reported')
    ws[a('Site "engineer" reported (incl. descriptions)')] = r.get('webrep_eng'); ws[a('Site "verification" reported (incl. descriptions)')] = r.get('webrep_ver')
    ws[a('Careers page used')] = r.get('web_url', r['url'])
    for h in ['Total: scraper/site', 'Engineer: scraper/site', 'Verification: scraper/site', 'Title overlap', 'Checks matching', 'Checks available', 'Status', 'Notes', 'Scraper error', 'Site error / note',
              'Site "engineer" reported (incl. descriptions)', 'Site "verification" reported (incl. descriptions)', 'How the site total was read', 'Site stated total (raw, not always trustworthy)', 'Careers page used']:
        ws[a(h)].font = body; ws[a(h)].border = box
    for h in ['Total: scraper/site', 'Engineer: scraper/site', 'Verification: scraper/site', 'Title overlap']: ws[a(h)].number_format = '0%'
    ws[a('Status')].font = bold
last = len(rows) + 1
ws.freeze_panes = 'B2'; ws.auto_filter.ref = f'A1:{get_column_letter(len(cols))}{last}'
st = f'{L["Status"]}2:{L["Status"]}{last}'
for text_, color in [('Working', 'C6EFCE'), ('Likely working', 'E2EFDA'), ('Partially working', 'FFEB9C'), ('Not working', 'FFC7CE'), ('Cannot verify', 'D9D9D9')]:
    ws.conditional_formatting.add(st, CellIsRule(operator='equal', formula=[f'"{text_}"'], fill=PatternFill('solid', bgColor=color, fgColor=color)))

# ---------------- Summary ----------------
sm = wb.create_sheet('Summary', 0)
sm.column_dimensions['A'].width = 30
for col in 'BCDEFGH': sm.column_dimensions[col].width = 16
sm['A1'] = 'Are the supported companies scraped correctly?'; sm['A1'].font = Font(name=F, size=14, bold=True)
sm['A2'] = 'Scraper results compared with the same searches on each company\'s own careers page. See Method for how and Companies for every number.'; sm['A2'].font = body
sm['A4'] = 'Status'; sm['B4'] = 'Companies'; sm['C4'] = 'Share'; sm['D4'] = 'Default-list companies'
for c in 'ABCD': sm[f'{c}4'].font = hdr_font; sm[f'{c}4'].fill = hdr_fill; sm[f'{c}4'].alignment = Alignment(horizontal='center')
rng = f'Companies!${L["Status"]}$2:${L["Status"]}${last}'; dl = f'Companies!$B$2:$B${last}'
for i, s_ in enumerate(['Working', 'Likely working', 'Partially working', 'Not working', 'Cannot verify']):
    r = 5 + i; sm[f'A{r}'] = s_; sm[f'B{r}'] = f'=COUNTIF({rng},A{r})'; sm[f'C{r}'] = f'=B{r}/$B$10'; sm[f'D{r}'] = f'=COUNTIFS({rng},A{r},{dl},"yes")'
    sm[f'C{r}'].number_format = '0%'
    for c in 'ABCD': sm[f'{c}{r}'].font = body; sm[f'{c}{r}'].border = box
sm['A10'] = 'All supported companies'; sm['B10'] = '=SUM(B5:B9)'; sm['C10'] = '=SUM(C5:C9)'; sm['D10'] = '=SUM(D5:D9)'; sm['C10'].number_format = '0%'
for c in 'ABCD': sm[f'{c}10'].font = bold; sm[f'{c}10'].border = box
sm['A12'] = 'By adapter'; sm['A12'].font = bold
for i, h in enumerate(['Adapter', 'Working', 'Likely working', 'Partially working', 'Not working', 'Cannot verify', 'Total']):
    c = sm.cell(row=13, column=1 + i, value=h); c.font = hdr_font; c.fill = hdr_fill; c.alignment = Alignment(horizontal='center', wrap_text=True)
adapters = sorted({r['adapter'] for r in rows}, key=lambda a: (-sum(1 for r in rows if r['adapter'] == a), a))
ad = f'Companies!$C$2:$C${last}'
for i, a_ in enumerate(adapters):
    r = 14 + i; sm[f'A{r}'] = a_
    for j, s_ in enumerate(['Working', 'Likely working', 'Partially working', 'Not working', 'Cannot verify']):
        sm.cell(row=r, column=2 + j, value=f'=COUNTIFS({ad},$A{r},{rng},"{s_}")')
    sm[f'G{r}'] = f'=SUM(B{r}:F{r})'
    for c in 'ABCDEFG': sm[f'{c}{r}'].font = body; sm[f'{c}{r}'].border = box
e = 14 + len(adapters)
sm[f'A{e}'] = 'Total'; 
for c in 'BCDEFG': sm[f'{c}{e}'] = f'=SUM({c}14:{c}{e-1})'
for c in 'ABCDEFG': sm[f'{c}{e}'].font = bold; sm[f'{c}{e}'].border = box
sm[f'A{e+2}'] = 'Jobs'; sm[f'A{e+2}'].font = bold
sm[f'A{e+3}'] = 'Jobs the scraper read, all companies'; sm[f'B{e+3}'] = f'=SUM(Companies!$G$2:$G${last})'
sm[f'A{e+4}'] = 'Jobs the scraper read, default list'; sm[f'B{e+4}'] = f'=SUMIFS(Companies!$G$2:$G${last},{dl},"yes")'
for r in (e+3, e+4): sm[f'A{r}'].font = body; sm[f'B{r}'].font = body; sm[f'B{r}'].number_format = '#,##0'
wb.save(sys.argv[1] if len(sys.argv) > 1 else f'{ROOT}/docs/scrape-audit/scraper-vs-website.xlsx')
print('written', len(rows))

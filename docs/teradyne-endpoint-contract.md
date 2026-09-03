# Teradyne – Endpoint Contract

## Employer and Board

**Company:** Teradyne, Inc.
**Official Careers Page:** https://www.teradyne.com/company/careers/
**Job Board:** https://jobs.teradyne.com/teradyne

## Platform

**Platform Name:** iCIMS / J2W (with SAP SuccessFactors backend resources)

Teradyne uses the iCIMS platform (J2W variant) for job board management, which includes references to SAP SuccessFactors (hcm41.sapsf.com) for resource loading.

## Listing Endpoint Attempt

The standard iCIMS listing endpoint does not return JSON data. Attempted endpoints:

**Primary Attempt:**
- **URL:** `https://jobs.teradyne.com/teradyne/jobs/search`
- **Method:** GET with query parameters: `search=`, `page=1`, `pageSize=20`
- **Expected by iCIMS adapter:** JSON response with jobs array
- **Actual Result:** Returns HTML page (not JSON)

## Alternative Endpoints Tested

Attempted alternative paths that might expose API:
- `/api/jobs` → HTML response
- `/api/search` → HTML response
- `/Search/Jobs` → HTML response
- `/api/job-listings` → HTML response
- `/Jobs/List` → HTML response
- `/Jobs/GetJobs` → HTML response
- `/search/` → HTML response

All return HTML, not JSON.

## Response Structure

The jobs page returns HTML with:
- Server-side rendered content using J2W (iCIMS) framework
- Handlebars/Mustache-like templating
- Links to external resources from SAP SuccessFactors
- JavaScript-driven client-side rendering

No JSON endpoint identified that returns job listings in structured format.

## robots.txt

```
User-agent: *
Disallow: /applybutton/
Disallow: /talentcommunity/
Disallow: /mobile/talentcommunity/
Disallow: /emailsubscribe/
Disallow: /email/image/
Disallow: /services/
Disallow: /preapply/
Disallow: /error
Disallow: /unsubscribe/
Disallow: /reset/
```

The `/teradyne/` path is **allowed** for scraping. However, `/services/` is disallowed, which likely contains API endpoints.

## Technical Challenges

1. **No JSON API:** iCIMS endpoint not returning JSON despite being standard iCIMS platform
2. **JavaScript-rendered:** Page content is dynamically loaded on client side
3. **Service endpoints blocked:** `/services/` is disallowed by robots.txt, suggesting API endpoints are there but explicitly blocked for automated access
4. **Mixed backend:** Uses SAP SuccessFactors resources, indicating complex backend integration

## Current Status

The iCIMS platform typically supports a JSON API at `/jobs/search`, but Teradyne's implementation:
- Does not expose this endpoint as JSON
- Serves HTML instead
- Blocks the `/services/` path where APIs typically live
- Relies on client-side JavaScript rendering

## Recommendation

Teradyne would require:
1. Custom iCIMS adapter that can parse HTML responses (if jobs are embedded in page)
2. Or contact Teradyne to enable JSON API endpoints
3. Or reverse-engineer the client-side JavaScript to find the actual data loading mechanism
4. Monitor if robots.txt restrictions are intentional or can be negotiated

The blocked `/services/` path and HTML-only responses suggest Teradyne may not want automated job scraping via API.

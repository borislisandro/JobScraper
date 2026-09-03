# BESI – Endpoint Contract

## Employer and Board

**Company:** BESI (BE Semiconductor Industries N.V.)
**Official Careers Page:** https://www.besi.com/careers
**Job Board:** https://besi.kandidatenportal.eu/

## Platform

**Platform Name:** Kandidatenportal (German job portal platform, custom system)

## Listing Endpoint

The job listings are embedded in the HTML response from the `/Jobs` page as server-side rendered content within a JavaScript initialization object.

**URL:** `https://besi.kandidatenportal.eu/Jobs`
**Method:** GET
**Headers:** Standard HTTP (Content-Type: text/html)
**Body:** None

## Response Structure

The jobs data is embedded in a `<script>` tag as part of the JobList initialization. The response is HTML with an embedded JSON-like object containing:

**Top-level keys in the response:**
- `RegionsViewModel` (filter options)
- `JobProfiles` (filter options)
- `FilteredJobProfiles`
- `FilteredText`
- `Jobs` (array of job listings)
- `TotalJobsCount` (number)
- `DisplayJobProfileFilter` (boolean)
- `DisplayRegionFilter` (boolean)
- `DisplayFilter` (boolean)
- `DisplayJobDate` (boolean)
- `DisplayJobLocation` (boolean)
- `DisplayCompanyPictures` (boolean)
- `Pagination` (pagination info)
- `CurrentLanguage` (string)
- `DisplayJobMap` (boolean)
- `EnableSEOFriendlyUrls` (boolean)

**Job object structure:**
```json
{
  "Id": 268620,
  "Title": "(Senior) Electrical Design Engineer f/m/d",
  "SubTitle": "",
  "Location": "Radfeld / Tirol",
  "Date": "02.09.2026",
  "UrlEncodedTitle": "(Senior)-Electrical-Design-Engineer-f-m-d",
  "CompanyPicture": null,
  "PortalUrl": null,
  "OnlineDateCorrected": "/Date(1788300000000)/",
  "GeoLocation": null,
  "SocialMedia": {
    "Title": "...",
    "Description": "...",
    "Image": "..."
  }
}
```

## Pagination

**Current State:** No pagination API. The page appears to load all jobs at once in the initial page load.
- `Pagination.DisplayPagination`: false (pagination UI disabled)
- `Pagination.IsPagination`: false
- Total jobs count: 9 (at time of investigation)

To support pagination, would need to identify if backend supports page parameters.

## Description Location

Descriptions are **not** included in the listing response. Each job has an individual job detail page:

**Per-job endpoint:** `https://besi.kandidatenportal.eu/Job/{id}`

Example: https://besi.kandidatenportal.eu/job/268620

Descriptions must be fetched from the individual job detail pages.

## robots.txt

```
User-agent: *
Disallow: /Login/
Disallow: /Register/
```

The `/Jobs` path and `/Job/{id}` paths are **allowed** for scraping. No override needed.

## Technical Challenges

1. **No JSON API:** Jobs are embedded in server-rendered HTML, not available through a dedicated JSON endpoint
2. **Client-side rendering:** The page uses Mustache templates with JavaScript to render the job list
3. **Pagination unclear:** Current page shows no pagination, but may load all jobs upfront
4. **Description extraction:** Requires fetching individual job detail pages for full job descriptions

## Recommendation

Kandidatenportal would require either:
- HTML scraping/parsing to extract embedded job data from the page
- Or wait for Kandidatenportal to expose a JSON API

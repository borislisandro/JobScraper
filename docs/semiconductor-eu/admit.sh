#!/bin/bash
# docs/semiconductor-eu/admit.sh <id> <trackerName> <country> <tags> <careersUrl>   (tracker row is created when missing)
cd "$(dirname "$0")/../.." && id=$1 && shift
node scripts/admit-company.mjs "$id" docs/company-proofs/$id.json --create-row --country "$2" --tags "$3" --careers "$4" --notes "$(node -e 'const r=require("./docs/company-candidates/'$id'.json").discovery?.relevance;console.log(r?`Semiconductor EU batch: ${r.roleTitles} chip-design titles on ${r.boardJobs} rows (${r.roleTitlesInTargetCountries} in target countries) on ${r.checkedAt}.`:"")')"

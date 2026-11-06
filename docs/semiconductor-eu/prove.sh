#!/bin/bash
# Full proof for one candidate: docs/semiconductor-eu/prove.sh <id>
cd "$(dirname "$0")/../.." && node scripts/verify-company-catalog.mjs --full --candidate docs/company-candidates/$1.json --report docs/company-proofs/$1.json 2>&1 | tail -3

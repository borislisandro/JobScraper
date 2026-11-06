// Promotes catalog boards into the default (starter) semiconductor list and bumps the pack version so
// existing installs re-seed.   node docs/semiconductor-eu/promote.mjs <version>
// Chosen on 2026-11-06 from live counts of DV/DFT/RTL/ASIC/FPGA/SoC titles in PT, ES, FR, NL, IE, AT, DE, UK, CH
// (see docs/semiconductor-eu/relevance.md); big mixed boards (IonQ, Teledyne, Philips, Bosch, CERN) are admitted but not promoted.
import {readFileSync,writeFileSync}from "node:fs";
const version=process.argv[2];if(!version)throw Error("Usage: promote.mjs <version>");
const promote=["Synopsys","Infineon","Graphcore","Cirrus Logic","Tenstorrent","Siemens","Monolithic Power Systems","Rambus","Imagination Technologies","Renesas","Nordic Semiconductor","SiPearl","Cadence",
 "Semidynamics","Fractile","Arago","Baya Systems","Riverlane","Olix","Pragmatic Semiconductor","Dolphin Semiconductor","Prophesee",
 "Openchip","Codasip","Raspberry Pi","Axiomise","Racyics","Arteris"];
const url=new URL("../../sidecar/company-catalog.json",import.meta.url),catalog=JSON.parse(readFileSync(url,"utf8"));
for(const name of promote){const company=catalog.companies.find(entry=>entry.name===name);if(!company)throw Error(`Not in the catalog: ${name}`);
 company.starter=true;if(!company.tags.includes("semiconductor"))company.tags=[...company.tags,"semiconductor"].sort()}
catalog.version=version;
writeFileSync(url,JSON.stringify(catalog,null,1)+"\n");
console.log(`${catalog.companies.filter(company=>company.starter).length} starters, version ${version}`);

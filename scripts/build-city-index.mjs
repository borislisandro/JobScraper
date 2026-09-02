// Regenerates the bundled city index from GeoNames. Run by hand when the data should be refreshed;
// the app itself never fetches anything, and the generated files are checked in.
//
//   node scripts/build-city-index.mjs
//
// Source: https://download.geonames.org/export/dump/  (cities1000.zip, countryInfo.txt)
// Licence: Creative Commons Attribution 4.0. The app credits GeoNames on the Diagnostics tab.
//
// Output, all generated, all checked in:
//   src-tauri/data/cities.tsv     folded city name -> the countries that have one, with populations
//   src-tauri/data/countries.tsv  folded country name and alias -> ISO2
//   src-tauri/data/fold.tsv       every accented letter -> its plain form, so both languages fold alike
//   src/countries.ts              the picker's list: ISO2 + display name
import {writeFileSync,mkdirSync} from "node:fs";import {inflateRawSync} from "node:zlib";
const DUMP="https://download.geonames.org/export/dump";
// Everything either side of this file folds names the same way: no accents, no punctuation, one
// space between words. src-tauri/src/locations.rs::fold is the Rust half and must not drift.
const SPECIALS={"ø":"o","æ":"ae","œ":"oe","ł":"l","đ":"d","ß":"ss","þ":"th","ð":"d","ı":"i","ħ":"h","ŋ":"n","ŧ":"t","ʼ":"","'":""};
export const fold=value=>String(value??"").normalize("NFD").replace(/[̀-ͯ]/g,"").toLowerCase()
 .replace(/[øæœłđßþðıħŋŧʼ']/g,c=>SPECIALS[c]??c).replace(/[^a-z0-9]+/g," ").trim();
// A zip holding one file: the local header says how long its name and extra field are, and the
// payload after them is a raw deflate stream. No dependency needed for that.
const unzipSingle=buffer=>{
 if(buffer.readUInt32LE(0)!==0x04034b50)throw Error("not a zip file");
 const start=30+buffer.readUInt16LE(26)+buffer.readUInt16LE(28);
 return inflateRawSync(buffer.subarray(start)).toString("utf8")};
const download=async(name,zipped)=>{
 process.stdout.write(`fetching ${name}… `);
 const response=await fetch(`${DUMP}/${name}`);
 if(!response.ok)throw Error(`${name}: HTTP ${response.status}`);
 const body=Buffer.from(await response.arrayBuffer());
 process.stdout.write(`${(body.length/1e6).toFixed(1)}MB\n`);
 return zipped?unzipSingle(body):body.toString("utf8")};

// An English exonym is what a job board writes ("Munich", not "München"), so the alternate names
// are worth keeping — but that column also holds every script on earth plus airport codes and
// postal abbreviations. Keep only plain Latin names of a plausible length, a few per city.
// Exonyms only earn their bytes for places big enough that boards write them in English at all;
// below that, the local spelling is what a careers page uses anyway. Gates the file at ~3MB
// instead of ~8MB with no loss on any city a job is likely to name.
// The alternate-name column is ordered by nothing useful and runs to dozens of languages, so the
// whole list is read and the closest spellings win: GeoNames calls the place "Munich", and the name
// a German careers page writes, "München", sits far down its alternates. Ranking by how much of the
// primary name a candidate still shares keeps that one and drops "Monaco di Baviera".
const ALT_MIN_POPULATION=20000,altLimit=population=>population>=200000?6:3;
const sharedPrefix=(a,b)=>{let i=0;while(i<a.length&&i<b.length&&a[i]===b[i])i++;return i};
const usefulAlternates=(raw,taken,population,primary)=>{
 if(population<ALT_MIN_POPULATION)return [];
 const scored=new Map();
 for(const candidate of String(raw||"").split(",")){
  const name=candidate.trim();
  if(name.length<4||name.length>40)continue;
  if(!/^[A-Za-zÀ-ÿ' .-]+$/.test(name))continue;   // one script only, no digits, no parentheses
  if(name===name.toUpperCase())continue;          // LIS, NYC and friends are codes, not names
  const folded=fold(name);
  if(!folded||folded.length<4||taken.has(folded)||scored.has(folded))continue;
  scored.set(folded,sharedPrefix(folded,primary))}
 const out=[...scored.entries()].sort((a,b)=>b[1]-a[1]).slice(0,altLimit(population)).map(([name])=>name);
 for(const name of out)taken.add(name);
 return out};

// Country names, their ISO codes, and the handful of aliases boards actually write. GeoNames names
// the state ("United States"), a careers page names the habit ("USA", "U.S.", "UK").
const ALIASES={us:["usa","us","u s a","u s","united states of america","america"],gb:["uk","u k","great britain","britain","england","scotland","wales","northern ireland"],
 kr:["south korea","republic of korea","korea south"],kp:["north korea"],cz:["czech republic","czechia"],nl:["holland","the netherlands"],ae:["uae","u a e"],
 ru:["russian federation"],vn:["viet nam"],tw:["taiwan roc","chinese taipei"],cn:["mainland china","prc"],ie:["republic of ireland","eire"],
 tr:["turkiye"],mk:["north macedonia"],ci:["ivory coast"],cv:["cape verde"],mm:["burma"],la:["laos"],sy:["syria"],bo:["bolivia"],ve:["venezuela"],
 tz:["tanzania"],md:["moldova"],ir:["iran"],be:["belgium"],ch:["switzerland"]};

// GeoNames names most large cities in English and ships only a thin slice of their alternates in
// the cities files, so the local spelling a careers page uses is often absent altogether. Rather
// than pull the 500MB alternate-name dump for a few dozen places, the everyday second spellings are
// listed here and copied from the row they belong to. Alias on the left, GeoNames' own name on the
// right; an alias whose target is missing is simply skipped.
const CITY_ALIASES={munchen:"munich",muenchen:"munich",koln:"cologne",koeln:"cologne",nurnberg:"nuremberg",nuernberg:"nuremberg",
 wien:"vienna",praha:"prague",warszawa:"warsaw",lisboa:"lisbon",oporto:"porto",sevilla:"seville",
 milano:"milan",roma:"rome",torino:"turin",napoli:"naples",firenze:"florence",venezia:"venice",genova:"genoa",
 bruxelles:"brussels",brussel:"brussels",antwerpen:"antwerp",gent:"ghent","den haag":"the hague","s gravenhage":"the hague",
 kobenhavn:"copenhagen",koebenhavn:"copenhagen",goteborg:"gothenburg",goeteborg:"gothenburg",
 moskva:"moscow",bucuresti:"bucharest",athina:"athens",athinai:"athens",beograd:"belgrade",lisbonne:"lisbon",
 bangalore:"bengaluru",bombay:"mumbai",madras:"chennai",calcutta:"kolkata",gurgaon:"gurugram",poona:"pune",
 peking:"beijing",canton:"guangzhou",saigon:"ho chi minh city",bengaluru:"bengaluru","yokneam ilit":"yokneam illit",yokneam:"yokneam illit"};
const [cities,countryInfo]=await Promise.all([download("cities1000.zip",true),download("countryInfo.txt",false)]);

const countries=[],countryTerms=new Map();
for(const line of countryInfo.split("\n")){
 if(!line||line.startsWith("#"))continue;
 const cell=line.split("\t"),code=cell[0]?.trim().toLowerCase(),name=cell[4]?.trim();
 if(!code||code.length!==2||!name)continue;
 countries.push({code:code.toUpperCase(),name});
 for(const term of [name,name.replace(/^the /i,""),...(ALIASES[code]||[])]){const folded=fold(term);if(folded&&!countryTerms.has(folded))countryTerms.set(folded,code.toUpperCase())}}

// One row per folded name; a name shared by several countries keeps them all, most populous first,
// so the resolver can prefer the likely one and still recognise the others when a country is named.
const index=new Map();
let rows=0;
for(const line of cities.split("\n")){
 if(!line)continue;
 const cell=line.split("\t");
 const country=cell[8]?.trim().toUpperCase(),population=Number(cell[14]||0);
 if(!country||country.length!==2)continue;
 rows++;
 const taken=new Set();
 const names=[cell[1],cell[2]].map(fold).filter(name=>name.length>=3&&!taken.has(name)&&taken.add(name));
 for(const name of [...names,...usefulAlternates(cell[3],taken,population,names[0]||"")]){
  const byCountry=index.get(name)||new Map();
  // The same name inside one country (three Springfields) counts as its largest.
  byCountry.set(country,Math.max(byCountry.get(country)||0,population));
  index.set(name,byCountry)}}

let aliased=0;
for(const [alias,canonical] of Object.entries(CITY_ALIASES)){
 const row=index.get(canonical);
 if(!row||index.has(alias))continue;
 index.set(alias,row);aliased++}

const lines=[...index.entries()].sort(([a],[b])=>a<b?-1:a>b?1:0)
 .map(([name,byCountry])=>`${name}\t${[...byCountry.entries()].sort((a,b)=>b[1]-a[1]).map(([code,population])=>`${code}:${population}`).join(",")}`);
mkdirSync("src-tauri/data",{recursive:true});
writeFileSync("src-tauri/data/cities.tsv",`${lines.join("\n")}\n`);
writeFileSync("src-tauri/data/countries.tsv",`${[...countryTerms].map(([term,code])=>`${term}\t${code}`).join("\n")}\n`);
writeFileSync("src/countries.ts",`// Generated by scripts/build-city-index.mjs from GeoNames countryInfo.txt (CC BY 4.0).
// The picker's list. City names live in src-tauri/data/cities.tsv and are resolved to these codes
// once, when a job is stored — see src-tauri/src/locations.rs.
export type Country={code:string;name:string};
export const COUNTRIES:Country[]=${JSON.stringify(countries.sort((a,b)=>a.name<b.name?-1:1))};
`);
// The Rust half of fold() cannot call NFD, so the table it needs is emitted from the JavaScript
// half that can. Generated together, they cannot drift apart.
const folds=[];
for(let code=0xa0;code<=0x2af;code++){const letter=String.fromCodePoint(code),plain=fold(letter);
 if(plain&&plain!==letter.toLowerCase())folds.push(`${letter}\t${plain}`)}
writeFileSync("src-tauri/data/fold.tsv",`${folds.join("\n")}\n`);
console.log(`${rows} city rows -> ${lines.length} names (${aliased} aliased), ${countries.length} countries, ${folds.length} folded letters`);

// Builds the self-contained jobs page (data embedded, no network needed) from the TSV of Portugal data/AI vacancies.
//   node docs/ds-portugal/build-jobs-page.mjs
import {readFileSync,writeFileSync}from "node:fs";
const dir=new URL(".",import.meta.url);
// Source of truth is the full scrape; senior roles are dropped here so every output format agrees.
const SENIOR=/\bs[eé]nior\b/i,clean=value=>String(value||"").replace(/\s+/g," ").trim();
const all=readFileSync(new URL("pt-jobs-data-ai.tsv",dir),"utf8").trim().split("\n").slice(1).map(line=>{
 const [,company,title,location,posted,link,via]=line.split("\t");
 return{c:clean(company),t:clean(title),l:clean(location),p:/^\d{4}-\d\d-\d\d/.test(posted)?posted.slice(0,10):"",u:clean(link),v:clean(via)}});
const rows=all.filter(job=>!SENIOR.test(job.t)).sort((a,b)=>a.c.localeCompare(b.c)||a.t.localeCompare(b.t));
// Plain-text exports of the same rows.
const columns=["company","title","location","posted","link","found_via"],cells=job=>[job.c,job.t,job.l,job.p,job.u,job.v];
writeFileSync(new URL("data-science-ai-jobs-portugal.tsv",dir),[columns.join("\t"),...rows.map(job=>cells(job).join("\t"))].join("\n")+"\n");
const quote=value=>`"${String(value).replace(/"/g,'""')}"`;
writeFileSync(new URL("data-science-ai-jobs-portugal.csv",dir),"\ufeff"+[columns.join(","),...rows.map(job=>cells(job).map(quote).join(","))].join("\r\n")+"\r\n");
// "<" is escaped so no title can close the script element that carries the data.
const data=JSON.stringify(rows).replace(/</g,"\\u003c");
const html=`<!doctype html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Data science and AI jobs in Portugal</title>
<style>
:root{--bg:#0e1117;--surface:#161b22;--surface2:#1c232d;--border:#2a3340;--text:#e6e9ef;--muted:#9aa4b2;--accent:#6cb2ff;--accent-bg:#14304f;--chip:#1c232d;--ok:#5fd38d;--warn:#e3b341;color-scheme:dark}
:root[data-theme="light"]{--bg:#f6f7f9;--surface:#fff;--surface2:#eef1f5;--border:#d5dae2;--text:#1b2230;--muted:#5b6678;--accent:#0b63c5;--accent-bg:#dcebfb;--chip:#eef1f5;--ok:#17803d;--warn:#8a6100;color-scheme:light}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
.wrap{max-width:1100px;margin:0 auto;padding:20px 16px 48px}
header{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;flex-wrap:wrap}
h1{margin:0;font-size:1.45rem;letter-spacing:-.01em}
.sub{margin:4px 0 0;color:var(--muted);font-size:.9rem}
button,select,input{font:inherit;color:inherit}
.theme{background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:7px 12px;cursor:pointer}
.panel{margin:18px 0 12px;background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:14px;display:grid;gap:12px}
.row{display:flex;gap:10px;flex-wrap:wrap;align-items:center}
.label{font-size:.78rem;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);min-width:64px}
input[type=search],select{background:var(--surface2);border:1px solid var(--border);border-radius:8px;padding:8px 10px;min-height:38px}
input[type=search]{flex:1 1 220px}
.chip{background:var(--chip);border:1px solid var(--border);border-radius:999px;padding:6px 13px;cursor:pointer;min-height:34px}
.chip[aria-pressed="true"]{background:var(--accent-bg);border-color:var(--accent);color:var(--accent);font-weight:600}
.chip .n{opacity:.7;font-weight:400;margin-left:5px;font-size:.85em}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.count{margin:0 0 10px;color:var(--muted)}
ul.jobs{list-style:none;margin:0;padding:0;display:grid;gap:8px}
li.job{background:var(--surface);border:1px solid var(--border);border-radius:10px;padding:12px 14px;display:grid;grid-template-columns:1fr auto;gap:2px 16px}
li.job:hover{border-color:var(--accent)}
.title{font-weight:600;font-size:1rem;color:var(--accent);text-decoration:none;overflow-wrap:anywhere}
.title:hover{text-decoration:underline}
.meta{color:var(--muted);font-size:.88rem;grid-column:1}
.meta b{color:var(--text);font-weight:600}
.when{grid-row:1/span 2;grid-column:2;text-align:right;font-size:.85rem;color:var(--muted);align-self:center;white-space:nowrap}
.when .fresh{color:var(--ok);font-weight:600}
.when .old{color:var(--warn)}
.empty{padding:28px;text-align:center;color:var(--muted);background:var(--surface);border:1px dashed var(--border);border-radius:10px}
.note{margin-top:22px;color:var(--muted);font-size:.82rem}
@media(max-width:560px){li.job{grid-template-columns:1fr}.when{grid-row:auto;grid-column:1;text-align:left;margin-top:4px}.label{min-width:100%}}
</style>
</head>
<body>
<div class="wrap">
<header>
 <div><h1>Data science and AI jobs in Portugal</h1><p class="sub" id="sub"></p></div>
 <button class="theme" id="theme" type="button" aria-label="Switch between dark and light theme"></button>
</header>
<section class="panel" aria-label="Filters">
 <div class="row"><input id="q" type="search" placeholder="Search title or company" aria-label="Search title or company"></div>
 <div class="row"><span class="label">City</span><span id="cities" class="row"></span></div>
 <div class="row"><span class="label">Role</span><span id="kinds" class="row"></span></div>
 <div class="row">
  <label class="label" for="company">Company</label><select id="company"></select>
  <label class="label" for="within">Posted</label>
  <select id="within"><option value="0">Any time</option><option value="7">Last 7 days</option><option value="30">Last 30 days</option><option value="90">Last 90 days</option></select>
  <label class="label" for="sort">Sort by</label>
  <select id="sort"><option value="new">Most recent</option><option value="old">Oldest first</option><option value="company">Company A to Z</option><option value="title">Title A to Z</option></select>
 </div>
</section>
<p class="count" id="count" aria-live="polite"></p>
<ul class="jobs" id="list"></ul>
<p class="note">Scraped from each employer's own job board on 2026-10-30. Cities come from the posting's location text; "Porto Salvo" is counted as Lisbon, where it is. Senior roles are left out. Postings without a date sort last when ordering by most recent. Check a posting is still open before applying.</p>
</div>
<script id="data" type="application/json">${data}</script>
<script>
(function(){
 var JOBS=JSON.parse(document.getElementById("data").textContent);
 var CITIES=[["porto","Porto"],["lisbon","Lisbon"],["aveiro","Aveiro area"],["braga","Braga"],["remote","Remote"],["other","Other / unspecified"]];
 var KINDS=[["ds","Data science"],["da","Data analysis / BI"],["ml","AI / ML"]];
 function citiesOf(loc){
  var s=String(loc).toLowerCase(),out=[];
  if(/lisbo|lisboa|oeiras|porto salvo|amadora|sintra|cascais|odivelas|loures|amoreiras|santos 37/.test(s))out.push("lisbon");
  var p=s.replace(/porto salvo|porto alegre/g,"");
  if(/\\bporto\\b|oporto|matosinhos|\\bmaia\\b|gaia|gondomar/.test(p))out.push("porto");
  if(/aveiro|ovar|[ií]lhavo|[aá]gueda/.test(s))out.push("aveiro");
  if(/braga|guimar/.test(s))out.push("braga");
  if(/remote|remoto/.test(s))out.push("remote");
  return out.length?out:["other"];
 }
 function kindsOf(t){
  var s=String(t).toLowerCase(),out=[];
  if(/data scien|scientist/.test(s))out.push("ds");
  if(/data analy|analytics|business intelligence|\\bbi\\b|power bi|analista/.test(s))out.push("da");
  if(/machine learning|\\bml\\b|\\bai\\b|artificial|llm|nlp|gen ?ai|deep learning|computer vision/.test(s))out.push("ml");
  return out;
 }
 JOBS.forEach(function(j){j.cities=citiesOf(j.l);j.kinds=kindsOf(j.t);j.ts=j.p?Date.parse(j.p+"T00:00:00Z"):null;j.hay=(j.t+" "+j.c).toLowerCase()});
 var state={q:"",cities:{},kinds:{},company:"",within:0,sort:"new"};
 var $=function(id){return document.getElementById(id)};
 var now=Date.now();
 function pill(group,key,label,box){
  var b=document.createElement("button");b.type="button";b.className="chip";b.setAttribute("aria-pressed","false");b.dataset.key=key;
  b.appendChild(document.createTextNode(label));var n=document.createElement("span");n.className="n";b.appendChild(n);
  b.addEventListener("click",function(){state[group][key]=!state[group][key];render()});box.appendChild(b);return b;
 }
 var cityBtn={},kindBtn={};
 CITIES.forEach(function(c){cityBtn[c[0]]=pill("cities",c[0],c[1],$("cities"))});
 KINDS.forEach(function(k){kindBtn[k[0]]=pill("kinds",k[0],k[1],$("kinds"))});
 var companies=Array.from(new Set(JOBS.map(function(j){return j.c}))).sort(function(a,b){return a.localeCompare(b)});
 var sel=$("company");sel.add(new Option("All companies",""));companies.forEach(function(c){sel.add(new Option(c,c))});
 function anyOn(o){return Object.keys(o).some(function(k){return o[k]})}
 function passes(j,skip){
  if(state.q&&j.hay.indexOf(state.q)<0)return false;
  if(skip!=="cities"&&anyOn(state.cities)&&!j.cities.some(function(c){return state.cities[c]}))return false;
  if(skip!=="kinds"&&anyOn(state.kinds)&&!j.kinds.some(function(k){return state.kinds[k]}))return false;
  if(state.company&&j.c!==state.company)return false;
  if(state.within){if(j.ts===null||(now-j.ts)/864e5>state.within)return false}
  return true;
 }
 function ago(j){
  if(j.ts===null)return null;var d=Math.max(0,Math.floor((now-j.ts)/864e5));
  var text=d===0?"today":d===1?"yesterday":d<60?d+" days ago":d<730?Math.round(d/30)+" months ago":Math.round(d/365)+" years ago";
  return{text:text,cls:d<=14?"fresh":d>180?"old":""};
 }
 function cmp(a,b){
  var byCo=a.c.localeCompare(b.c)||a.t.localeCompare(b.t);
  if(state.sort==="company")return byCo;
  if(state.sort==="title")return a.t.localeCompare(b.t)||a.c.localeCompare(b.c);
  if(a.ts===null&&b.ts===null)return byCo;if(a.ts===null)return 1;if(b.ts===null)return -1;
  return (state.sort==="old"?a.ts-b.ts:b.ts-a.ts)||byCo;
 }
 function render(){
  CITIES.forEach(function(c){var k=c[0],n=JOBS.filter(function(j){return passes(j,"cities")&&j.cities.indexOf(k)>=0}).length;
   cityBtn[k].setAttribute("aria-pressed",String(!!state.cities[k]));cityBtn[k].lastChild.textContent=n});
  KINDS.forEach(function(k){var key=k[0],n=JOBS.filter(function(j){return passes(j,"kinds")&&j.kinds.indexOf(key)>=0}).length;
   kindBtn[key].setAttribute("aria-pressed",String(!!state.kinds[key]));kindBtn[key].lastChild.textContent=n});
  var shown=JOBS.filter(function(j){return passes(j)}).sort(cmp);
  $("count").textContent=shown.length+" of "+JOBS.length+" jobs";
  var list=$("list");list.textContent="";
  if(!shown.length){var e=document.createElement("li");e.className="empty";e.textContent="No jobs match these filters.";list.appendChild(e);return}
  var frag=document.createDocumentFragment();
  shown.forEach(function(j){
   var li=document.createElement("li");li.className="job";
   var a=document.createElement("a");a.className="title";a.textContent=j.t;
   if(/^https?:\\/\\//.test(j.u)){a.href=j.u;a.target="_blank";a.rel="noopener noreferrer"}li.appendChild(a);
   var m=document.createElement("div");m.className="meta";var b=document.createElement("b");b.textContent=j.c;m.appendChild(b);
   m.appendChild(document.createTextNode(" · "+String(j.l).replace(/\\s+/g," ")+(j.v?" · via "+j.v.split(" - ")[0]:"")));li.appendChild(m);
   var w=document.createElement("div");w.className="when";var g=ago(j);
   if(g){var s=document.createElement("span");s.className=g.cls;s.textContent=g.text;w.appendChild(s);w.appendChild(document.createElement("br"));w.appendChild(document.createTextNode(j.p))}
   else w.textContent="no date";
   li.appendChild(w);frag.appendChild(li);
  });
  list.appendChild(frag);
 }
 $("q").addEventListener("input",function(e){state.q=e.target.value.trim().toLowerCase();render()});
 sel.addEventListener("change",function(e){state.company=e.target.value;render()});
 $("within").addEventListener("change",function(e){state.within=+e.target.value;render()});
 $("sort").addEventListener("change",function(e){state.sort=e.target.value;render()});
 var root=document.documentElement,tb=$("theme");
 function setTheme(t){root.setAttribute("data-theme",t);tb.textContent=t==="dark"?"Light theme":"Dark theme";try{localStorage.setItem("jobs-theme",t)}catch(e){}}
 var saved=null;try{saved=localStorage.getItem("jobs-theme")}catch(e){}
 setTheme(saved==="light"?"light":"dark");
 tb.addEventListener("click",function(){setTheme(root.getAttribute("data-theme")==="dark"?"light":"dark")});
 $("sub").textContent=JOBS.length+" data science, analytics, AI and ML postings, newest first. Click a title to open the posting.";
 render();
})();
</script>
</body>
</html>
`;
writeFileSync(new URL("data-science-ai-jobs-portugal.html",dir),html);
console.log(`${rows.length} jobs written (${all.length-rows.length} senior roles left out)`);

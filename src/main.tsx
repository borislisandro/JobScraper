import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { DndContext, useDraggable, useDroppable, type DragEndEvent } from "@dnd-kit/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { aggregateRows, formatDuration, latestSummary, phaseRows, recentFailures, requestKindRows } from "./performance-ui";
import { COUNTRIES } from "./countries";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { jobCountries } from "./api";
import { api, appLogs, backgroundSyncStatus, DEFAULT_POSTED_WINDOW, jobDescription, launchAtLoginStatus, POSTED_WINDOWS, purgeAllJobs, setBackgroundSync, setLaunchAtLogin, startupStatus, type StartupStatus, cancelScrape, cancelScrapeAll, captureSession, deleteBrowserSession, getSetting, setSetting, hasBrowserSession, scrapeAll, scrapePerformance, sourceConfig, startAutomaticSync, validateAttachmentSize, type Application, type Job, type JobDescription, type JobFilter, type Source, type WorkerEvent } from "./api";
import { listenForApplyConfirmation } from "./apply-prompts";
import { listenForScrapeEvents } from "./scrape-events";
import { describeFailure, describeLogEvent, describeOutcome, describeUnsupported, describeUpdateSummary, previewFromEvents, probeVerdict, supportsSessionCapture } from "./source-ui";
import { applicationStages, boardMoveError, isApplied, stageLabel } from "./application-ui";
import "./styles.css";

const queryClient=new QueryClient({defaultOptions:{queries:{refetchOnWindowFocus:false}}}); const pages=["Jobs","Sources","Applications","Diagnostics"] as const; type Page=typeof pages[number];
function useDebounced<T>(value:T,delay=250){const [settled,setSettled]=useState(value);useEffect(()=>{const timer=setTimeout(()=>setSettled(value),delay);return()=>clearTimeout(timer)},[value,delay]);return settled}
const elapsedSince=(started:number)=>{const seconds=Math.max(0,(Date.now()-started)/1000);return seconds<60?`${seconds.toFixed(1)}s`:`${Math.floor(seconds/60)}m ${Math.round(seconds%60)}s`};
const showSelect=(event:React.PointerEvent<HTMLSelectElement>)=>{if(event.button!==0||typeof event.currentTarget.showPicker!=="function")return;event.preventDefault();event.currentTarget.showPicker()};
const dragWindow=(event:React.MouseEvent<HTMLElement>)=>{if(event.button===0&&!(event.target as Element).closest("button"))void getCurrentWindow().startDragging()};
// The window exists before the backend has opened anything, so the first thing rendered is a
// splash rather than a blank frame. Startup pushes a "startup" event per phase; the poll is a
// belt-and-braces fallback so a missed event can never strand the splash forever.
// The window has no system frame, so it draws its own. Only these three commands are granted to
// the webview; the drag region is the titlebar itself, and double-clicking it maximises the way a
// real one does.
// Resolved inside the handlers, not at render: the same page opens in a plain browser during
// development, where there is no window to ask about and reaching for one throws.
function WindowControls(){
 return <div className="window-controls">
  <button type="button" aria-label="Minimise" onClick={()=>getCurrentWindow().minimize()}>
   <svg viewBox="0 0 10 10"><path d="M0 5h10"/></svg></button>
  <button type="button" aria-label="Maximise" onClick={()=>getCurrentWindow().toggleMaximize()}>
   <svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9"/></svg></button>
  <button type="button" className="close" aria-label="Close" onClick={()=>getCurrentWindow().close()}>
   <svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10"/></svg></button>
 </div>}
function StartupGate({children}:{children:React.ReactNode}){
 const [status,setStatus]=useState<StartupStatus>({phase:"opening",message:"Starting JobScraper…"});
 const done=status.phase==="ready"||status.phase==="failed";
 useEffect(()=>{let live=true;const stop=listen<StartupStatus>("startup",event=>{if(live)setStatus(event.payload)});
  return()=>{live=false;stop.then(off=>off()).catch(()=>{})}},[]);
 useEffect(()=>{if(done)return;let live=true;
  const refresh=()=>startupStatus().then(next=>{if(live)setStatus(next)}).catch(()=>{});
  refresh();const poll=setInterval(refresh,500);
  return()=>{live=false;clearInterval(poll)}},[done]);
 if(status.phase==="ready")return <>{children}</>;
 if(status.phase==="failed")return <main className="splash"><div className="splash-chrome" data-tauri-drag-region><WindowControls/></div><section className="card"><h1>JobScraper could not start</h1>
  <p className="error">{status.error||"Something went wrong while starting up."}</p>
  <p className="muted">Close this window and try again. If it keeps happening, this message says what needs fixing.</p></section></main>;
 return <main className="splash"><div className="splash-chrome" data-tauri-drag-region><WindowControls/></div><section className="card"><h1>Starting JobScraper…</h1>
  <p className="muted" aria-live="polite">{status.message||"Getting ready"}</p>
  <div className="spinner" role="progressbar" aria-label="Starting up"/></section></main>}
function App(){
 const [page,setPage]=useState<Page>("Jobs");const [visited,setVisited]=useState<Page[]>(["Jobs"]);
 const [attempt,setAttempt]=useState<{applicationId:string;attemptId:string}|undefined>();
 const [settingsOpen,setSettingsOpen]=useState(false);const [updating,setUpdating]=useState(false);const [updateStatus,setUpdateStatus]=useState("");
 // The tab badges are the only place the three counts appear together. Sources and applications
 // come from the same cached queries the pages use; the job count is whatever Jobs is showing
 // after its filters, so it is reported upwards rather than re-queried unfiltered.
 const [jobCount,setJobCount]=useState<number>();
 useEffect(()=>listenForApplyConfirmation(listen,event=>setAttempt(current=>current?.attemptId===event.attemptId?current:event)),[]);
 const qc=useQueryClient();
 const {data:allSources=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources,refetchOnWindowFocus:true});
 const {data:allApps=[]}=useQuery({queryKey:["apps"],queryFn:api.apps,refetchOnWindowFocus:true});
 const activeSources=allSources.filter(source=>source.kind!=="reference"&&source.enabled).length;
 const scrapableSources=allSources.filter(source=>source.kind!=="reference").length;
 const attention=allSources.filter(source=>source.kind!=="reference"&&!source.enabled&&source.disabledReason).length;
 const badges:Record<Page,string>={Jobs:jobCount===undefined?"":String(jobCount),Sources:scrapableSources?`${activeSources} of ${scrapableSources}`:"0",Applications:String(allApps.length),Diagnostics:attention?String(attention):""};
 const decide=(decision:"yes"|"no"|"not_yet")=>attempt&&api.applyDecision(attempt.applicationId,decision).then(()=>{setAttempt(undefined);qc.invalidateQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["apps"]})});
 // A page is mounted the first time it is opened and only hidden afterwards, never unmounted.
 // Scraping always continued in the backend when the user switched away — what was thrown away
 // was every trace of it in the window: the Updating…/Cancel state of Update Jobs, the running
 // state of Update this source, the test preview, and the finished message. Leaving the page
 // mounted keeps all of it, so leaving and coming back shows the run exactly where it got to.
 const views:Record<Page,React.ReactNode>={Jobs:<Jobs onCount={setJobCount}/>,Sources:<Sources/>,Applications:<Applications/>,Diagnostics:<Diagnostics/>};
 const open=(next:Page)=>{setPage(next);setVisited(current=>current.includes(next)?current:[...current,next])};
 const lastUpdate=allSources.map(source=>source.lastSuccessAt?new Date(source.lastSuccessAt).getTime():0).reduce((latest,value)=>Math.max(latest,value),0);
 const updateNow=()=>{const runId=crypto.randomUUID();setUpdating(true);setUpdateStatus("Updating sources");scrapeAll(runId,[],false)
  .then(summary=>{setUpdateStatus(describeUpdateSummary(summary));qc.invalidateQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["sources"]})})
  .catch(error=>setUpdateStatus(String(error))).finally(()=>setUpdating(false))};
 useEffect(()=>{const close=(event:KeyboardEvent)=>{if(event.key==="Escape")setSettingsOpen(false)};window.addEventListener("keydown",close);return()=>window.removeEventListener("keydown",close)},[]);
 return <main>
  <div className="titlebar" onMouseDown={dragWindow}><img className="mark" src="/logo.png" alt="" width={28} height={28} draggable={false}/><h1>JobScraper</h1><div className="spacer"/>
   <button className="icon" type="button" aria-label="Settings" title="Settings" onClick={()=>setSettingsOpen(true)}><svg viewBox="0 0 20 20"><circle cx="10" cy="10" r="2.6"/><path d="M10 2.2v2.1M10 15.7v2.1M17.8 10h-2.1M4.3 10H2.2M15.5 4.5l-1.5 1.5M6 14l-1.5 1.5M15.5 15.5L14 14M6 6L4.5 4.5"/></svg></button><WindowControls/></div>
  <nav aria-label="Main navigation">{pages.map(p=><button className={page===p?"active":""} onClick={()=>open(p)} key={p}>{p}{badges[p]&&<span className="badge">{badges[p]}</span>}</button>)}
   <span className="nav-right"><span className="count">{updateStatus||(lastUpdate?`Updated ${new Date(lastUpdate).toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"})}${attention?` · ${attention} source${attention===1?"":"s"} need attention`:""}`:"Not updated yet")}</span>
    <button className="small" disabled={updating} onClick={updateNow}>{updating?"Updating…":"Update now"}</button></span></nav>
  <div className="pane">{visited.map(p=><div key={p} hidden={p!==page}>{views[p]}</div>)}
   {attempt&&<dialog open className="confirm" aria-label="Application confirmation"><div className="sheet">
    <div className="dialog-body">
     <div className="actions"><span className="tag">CONFIRM</span><h2>Did you submit this application?</h2></div>
     <p>JobScraper opened the posting in your browser but never submits on your behalf. Answer so the board and the event log stay accurate.</p>
    </div>
    <footer><button className="link" onClick={()=>decide("not_yet")}>Not yet</button>
     <button onClick={()=>decide("no")}>No</button>
     <button className="primary" onClick={()=>decide("yes")}>Yes, submitted</button></footer>
   </div></dialog>}
   </div>
  <Settings open={settingsOpen} onClose={()=>setSettingsOpen(false)}/>
  </main>}

// ---------------------------------------------------------------------------------------------
// Jobs — the page the app opens on. Source chips only narrow the view; every followed source is
// synchronised automatically once startup has finished.
// ---------------------------------------------------------------------------------------------
// Which sources are being looked at right now. Ticking a chip used to write sources.enabled, so
// narrowing the list to one board also stopped every other board from being read at all, and
// putting them back meant re-ticking seventeen. Following a source is a decision that belongs to
// the Sources page; this is a throwaway view that starts as everything followed.
function SourcePicker({sources,shown,onShow}:{sources:Source[];shown:Set<string>;onShow:(ids:string[])=>void}){
 const [pickerOpen,setPickerOpen]=useState(false);const [query,setQuery]=useState("");
 const scrapable=sources.filter(s=>s.kind!=="reference"&&s.enabled);
 if(!sources.some(s=>s.kind!=="reference"))return <p className="muted">No sources yet. Add one on the Sources page to start collecting jobs.</p>;
 if(!scrapable.length)return <p className="muted">Every source is switched off. Turn one back on from the Sources page.</p>;
 const many=scrapable.length>10,selected=scrapable.filter(source=>shown.has(source.id));
 const visible=many?selected.slice(0,10):scrapable;
 const filtered=scrapable.filter(source=>source.name.toLowerCase().includes(query.trim().toLowerCase()));
 const toggle=(source:Source,checked:boolean)=>onShow(checked?[...shown,source.id]:[...shown].filter(id=>id!==source.id));
 return <fieldset className="chips"><legend><span className="lbl">Showing</span><span className="count">{shown.size} of {scrapable.length} sources</span>
   {many&&<button type="button" className="link" onClick={()=>setPickerOpen(true)}>Choose sources</button>}
   <button type="button" className="link" onClick={()=>onShow(scrapable.map(s=>s.id))}>Select all</button><button type="button" className="link" onClick={()=>onShow([])}>Clear</button></legend>
  {visible.map(s=><label className={shown.has(s.id)?"chip on":"chip"} key={s.id}>
   <input type="checkbox" checked={shown.has(s.id)} onChange={e=>toggle(s,e.target.checked)}/>{s.name}<span className="n">{s.jobCount.toLocaleString()}</span></label>)}
  {many&&<button type="button" className="chip more" onClick={()=>setPickerOpen(true)}>+{Math.max(0,selected.length-visible.length)} more shown · {scrapable.length-selected.length} hidden</button>}
  {pickerOpen&&<div className="picker" aria-label="Choose sources"><div className="top"><input value={query} onChange={event=>setQuery(event.target.value)} placeholder="Filter sources by name"/><span className="count">{shown.size} of {scrapable.length} selected</span></div>
   <div className="list">{filtered.map(source=><label key={source.id}><input type="checkbox" checked={shown.has(source.id)} onChange={event=>toggle(source,event.target.checked)}/><span>{source.name}</span><span className="n">{source.jobCount.toLocaleString()}</span></label>)}</div>
   <footer><span className="count">Source choices only change this view</span><button type="button" className="small" onClick={()=>onShow(scrapable.map(s=>s.id))}>Select all</button><button type="button" className="small" onClick={()=>onShow([])}>Clear</button><button type="button" className="small primary" onClick={()=>setPickerOpen(false)}>Done</button></footer></div>}
 </fieldset>}
function useJobActions(job:Job,onStatus:(message:string)=>void){
 const qc=useQueryClient();const [busy,setBusy]=useState(false);
 const refresh=()=>{qc.removeQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["apps"]})};
 const act=(open:boolean)=>{setBusy(true);
  // Save and Apply are the same save; Apply just also opens the posting. Saving twice is
  // harmless because the backend returns the existing application for a job.
  api.createApp(job.id).then(application=>open?api.openApply(application.id):undefined)
   .then(()=>{onStatus(open?`Opened ${job.title} in your browser and saved it to Applications.`:`Saved ${job.title} to Applications.`);refresh()})
   .catch(error=>onStatus(String(error))).finally(()=>setBusy(false))};
 return{busy,act};}
const jobSalary=(job:Job)=>job.salaryMin||job.salaryMax?`${job.salaryMin??"?"}–${job.salaryMax??"?"} ${job.salaryCurrency??""}`.trim():"not listed";
export const JobCard=React.memo(function JobCard({job,sourceName,onStatus,onOpen}:{job:Job;sourceName?:string;onStatus:(message:string)=>void;onOpen?:(job:Job)=>void}){
 const {busy,act}=useJobActions(job,onStatus);const stage=job.applicationStage;
 const gone=job.availability==="closed"?"Closed":job.availability==="possibly_closed"?"May be gone":null;
 const open=()=>onOpen?.(job);
 return <div className="job" role="button" tabIndex={0} onClick={open} onKeyDown={event=>{if(event.key==="Enter"||event.key===" ")open()}}>
   <span className="job-title">{job.title}</span>
   <span className="job-meta">{job.company}{sourceName&&sourceName!==job.company?` · ${sourceName}`:""}</span>
   <span className="job-meta dim hide-md">{job.location||"Location not listed"}</span>
   <span className="job-posted hide-sm">{job.postedAt||"not listed"}</span>
   <span className="job-meta dim only-wide">{jobSalary(job)}</span>
   <span className="job-state hide-sm">
    {gone&&<span className="pill" title={job.availability==="closed"
     ?"Two complete reads of this board did not list this job."
     :"The last complete read of this board did not list this job."}>{gone}</span>}
    {stage&&<span className={isApplied(stage)?"pill applied":"pill good"}>{isApplied(stage)?`Applied · ${stageLabel(stage)}`:"Saved"}</span>}
   </span>
   <span className="row-actions"><button className="small" disabled={busy} onClick={event=>{event.stopPropagation();if(stage)open();else act(false)}}>{stage?"Open":"Save"}</button>
    {!isApplied(stage)&&<button className="small primary" disabled={busy||!job.applyUrl} onClick={event=>{event.stopPropagation();act(true)}}>Apply</button>}</span>
  </div>});
function JobReader({jobs,index,sourceName,onMove,onClose,onStatus}:{jobs:Job[];index:number;sourceName:(job:Job)=>string|undefined;onMove:(index:number)=>void;onClose:()=>void;onStatus:(message:string)=>void}){
 const job=jobs[index];const [description,setDescription]=useState<JobDescription>();const {busy,act}=useJobActions(job,onStatus);
 const fetchDescription=()=>{setDescription(undefined);jobDescription(job.id).then(setDescription).catch(error=>setDescription({text:"",status:"failed",error:String(error)}))};
 useEffect(()=>{let active=true;setDescription(undefined);jobDescription(job.id).then(value=>active&&setDescription(value)).catch(error=>active&&setDescription({text:"",status:"failed",error:String(error)}));return()=>{active=false}},[job.id]);
 return <section className="reader" aria-label="Job details"><header><div className="top"><h2>{job.title}</h2><span className="nav-pair"><button className="small" disabled={!index} onClick={()=>onMove(index-1)}>↑ Previous</button><button className="small" disabled={index>=jobs.length-1} onClick={()=>onMove(index+1)}>↓ Next</button><button className="small" onClick={onClose}>Close</button></span></div>
   <div className="sub"><span>{job.company}</span><span>·</span><span>{job.location||"Location not listed"}</span><span>·</span><span>posted {job.postedAt||"date not listed"}</span></div></header>
  <div className="body"><div className="measure"><div className="facts"><div><span>Work mode</span><strong>{job.workMode||"not listed"}</strong></div><div><span>Seniority</span><strong>{job.seniority||"not listed"}</strong></div><div><span>Salary</span><strong>{jobSalary(job)}</strong></div><div><span>Source</span><strong>{sourceName(job)||job.company}</strong></div><div><span>Status</span><strong>{job.applicationStage?stageLabel(job.applicationStage):job.availability.replace("_"," ")}</strong></div></div>
   {description===undefined?<p className="description muted">Loading the description…</p>:description.status==="pending"?<p className="description muted">This listing has no downloadable description.</p>:description.status==="failed"?<p className="description warning">Description download failed{description.error?`: ${description.error}`:"."} <button className="link" onClick={fetchDescription}>Retry</button></p>:<p className="description">{description.text||"This listing published no description."}</p>}</div></div>
  <footer><button className="primary" disabled={busy||!job.applyUrl} onClick={()=>act(true)}>Apply — opens the posting</button>{!job.applicationStage&&<button disabled={busy} onClick={()=>act(false)}>Save for later</button>}{job.canonicalUrl&&<a className="link" href={job.canonicalUrl} target="_blank" rel="noreferrer">Open listing in browser</a>}<span className="spacer"/><span className="count">{index+1} of {jobs.length}</span></footer></section>}
function Jobs({onCount}:{onCount:(count:number)=>void}){
 const qc=useQueryClient();
 const [title,setTitle]=useState(""),[keyword,setKeyword]=useState(""),[sort,setSort]=useState<JobFilter["sort"]>("recent"),[savedOnly,setSavedOnly]=useState(false),[includeClosed,setIncludeClosed]=useState(false);
 // Countries are matched on the codes worked out when each job was stored, so this sends codes and
 // not city names. A listing whose country could not be worked out at all is left out while a
 // country filter is on — it is the one that would otherwise show up under every country.
 const [countries,setCountries]=useState<string[]>([]),[includeUnknownLocations,setIncludeUnknownLocations]=useState(false);
 // View state, so it resets to a year on every launch rather than remembering a narrow window
 // you set once and then wondered about.
 const [postedWithin,setPostedWithin]=useState<number|null>(DEFAULT_POSTED_WINDOW);
 // undefined means "not narrowed yet", which resolves to every followed source once they load.
 // Kept out of the database on purpose: this is where you are looking, not what you follow.
 const [shown,setShown]=useState<string[]>();
 const {data:sourceList=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources,refetchOnWindowFocus:true});
 const followed=sourceList.filter(s=>s.enabled&&s.kind!=="reference");
 const shownIds=shown??followed.map(s=>s.id);
 const countryName=(code:string)=>COUNTRIES.find(entry=>entry.code===code)?.name??code;
 // Only the countries the jobs on show are actually in. Offering all 250 states meant most of the
 // list led to an empty page, and it moves with the source selection: narrow the boards and the
 // countries narrow with them.
 const {data:[available=[],unplaced=0]=[]}=useQuery({queryKey:["job-countries",shownIds],queryFn:()=>jobCountries(shownIds)});
 const options=available.map(entry=>({...entry,name:countryName(entry.code)})).sort((a,b)=>a.name<b.name?-1:1);
 // A country that leaves the list when the sources change would otherwise stay selected invisibly,
 // filtering against something the picker no longer offers.
 useEffect(()=>{if(available.length)setCountries(current=>current.filter(code=>available.some(entry=>entry.code===code)))},[available]);
 // A plain dropdown: it opens on click, jumps as you type into it, and commits only when you pick.
 // The free-text box it replaced added a country on every keystroke, so "in" became India mid-word
 // and typing towards "Nigeria" stopped at "Niger".
 const addCountry=(code:string)=>{if(code)setCountries(current=>current.includes(code)?current:[...current,code])};
 const filter:JobFilter={title:useDebounced(title),keyword:useDebounced(keyword),sort,savedOnly,includeClosed,postedWithinDays:postedWithin,sourceIds:shownIds,countries,includeUnknownLocations};
 const {data,isLoading,fetchNextPage,hasNextPage,isFetchingNextPage}=useInfiniteQuery({queryKey:["jobs",filter],queryFn:({pageParam})=>api.jobs(filter,pageParam),initialPageParam:0,getNextPageParam:last=>last.hasMore?last.offset+last.items.length:undefined,refetchOnWindowFocus:true});
 const jobs=data?.pages.flatMap(page=>page.items)??[],total=data?.pages[0]?.total??0;
 const [status,setStatus]=useState("");const [batch,setBatch]=useState<string>();const automaticStarted=useRef(0);
 const [toolbarOpen,setToolbarOpen]=useState(true);const [readerIndex,setReaderIndex]=useState<number|null>(null);
 const names=new Map(sourceList.map(s=>[s.id,s.name]));
 const selected=followed.filter(s=>shownIds.includes(s.id));
 const automaticRun="automatic-startup";
 useEffect(()=>{let active=true;const stop=listen<{runId:string;phase?:string;completed?:number;current?:number;total?:number;source?:string;requests?:number;failed?:number;cancelled?:number}>("scrape-all-progress",event=>{
  const progress=event.payload;if(!active||progress.runId!==automaticRun)return;
  if(!automaticStarted.current)automaticStarted.current=Date.now();setBatch(automaticRun);
  const elapsed=automaticStarted.current?` · ${elapsedSince(automaticStarted.current)}`:"";
  if(!progress.phase){qc.invalidateQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["sources"]});const complete=(progress.completed??0)>=(progress.total??1);
   setStatus(complete?(progress.cancelled?`Update cancelled${elapsed}.`:`${progress.failed?`Update finished; ${progress.failed} source${progress.failed===1?"":"s"} could not be read${elapsed}.`:`Jobs are up to date${elapsed}.`} Descriptions download when opened.`):`Updating sources · ${progress.completed}/${progress.total}${elapsed}`);
   if(complete)setBatch(undefined)}
  if(progress.phase==="source-progress")setStatus(`Reading ${progress.source||"source"} · ${progress.requests||0} requests${elapsed}`);
 });stop.then(()=>active&&startAutomaticSync().catch(error=>setStatus(String(error))));return()=>{active=false;stop.then(unlisten=>unlisten()).catch(()=>{})}},[qc]);
 // "Filtered" means the view differs from the default one, which is what makes the empty state
 // honest: hiding closed jobs is the default, so turning them back on counts as a change too.
 const filtered=!!(title.trim()||keyword.trim()||savedOnly||includeClosed||countries.length||includeUnknownLocations||postedWithin!==DEFAULT_POSTED_WINDOW);
 const clearFilters=()=>{setTitle("");setKeyword("");setSavedOnly(false);setIncludeClosed(false);setCountries([]);setIncludeUnknownLocations(false);setPostedWithin(DEFAULT_POSTED_WINDOW)};
 // The nav badge counts what the page is showing, so narrowing the view moves it too.
 useEffect(()=>onCount(selected.length?total:0),[selected.length,total,onCount]);
 useEffect(()=>{const close=(event:KeyboardEvent)=>{if(event.key==="Escape")setReaderIndex(null)};window.addEventListener("keydown",close);return()=>window.removeEventListener("keydown",close)},[]);
 return <div className="view">
  <div className={toolbarOpen?"toolbar":"toolbar toolbar-collapsed"}>
   <header><div><h2>Jobs</h2><p>Sources update automatically when the app opens. The choices here only change what you are looking at.</p></div>
    <span className="count">{total?`${total} job${total===1?"":"s"}${jobs.length<total?` · showing ${jobs.length}`:""}`:""}</span>
    {batch&&<button className="small danger" onClick={()=>cancelScrapeAll(batch)}>Cancel update</button>}
    <button className="small" onClick={()=>setToolbarOpen(open=>!open)}>{toolbarOpen?"Hide filters":"Show filters"}</button></header>
   {!toolbarOpen&&<div className="filter-summary"><span>{filtered?<><b>Filters active</b> · {selected.length} sources</>:<><b>All recent jobs</b> · {selected.length} sources</>}</span><button className="link" onClick={()=>setToolbarOpen(true)}>Edit filters</button>{filtered&&<button className="link" onClick={clearFilters}>Clear</button>}</div>}
   <SourcePicker sources={sourceList} shown={new Set(shownIds)} onShow={setShown}/>
   <section className="filters" aria-label="Filters">
    <div className="section-label"><span className="lbl">Filters</span>{filtered&&<button type="button" className="link" onClick={clearFilters}>Clear all</button>}</div>
    <label className="grow"><span>Title contains</span><input value={title} onChange={e=>setTitle(e.target.value)} placeholder="verification"
     title="Searches job titles. Every word you type must appear; case and order are ignored."/></label>
    <label className="grow"><span>Anywhere in the listing</span><input value={keyword} onChange={e=>setKeyword(e.target.value)} placeholder="uvm, remote"
     title="Searches everything stored — title, company, location, and the description once you have opened a job. Every word must appear; case and order are ignored."/></label>
    <label><span>Posted within</span><select value={String(postedWithin)} onPointerDown={showSelect} onChange={e=>setPostedWithin(e.target.value==="null"?null:Number(e.target.value))}>
     {POSTED_WINDOWS.map(window=><option key={window.label} value={String(window.days)}>{window.label}</option>)}</select></label>
    <label><span>Countries</span>
     <select value="" disabled={!options.length} onPointerDown={showSelect} onChange={event=>addCountry(event.target.value)}>
      <option value="">{options.length?(countries.length?"Add another…":"Anywhere"):"Nothing to filter yet"}</option>
      {options.filter(entry=>!countries.includes(entry.code)).map(entry=>
       <option key={entry.code} value={entry.code}>{entry.name} ({entry.jobs})</option>)}</select></label>
    <label className="inline"><input type="checkbox" checked={savedOnly} onChange={e=>setSavedOnly(e.target.checked)}/> Saved and applied only</label>
    <label className="inline" title="Jobs their board has not listed in two complete reads are hidden by default."><input type="checkbox" checked={includeClosed} onChange={e=>setIncludeClosed(e.target.checked)}/> Include closed</label>
    {!!countries.length&&<div className="chosen"><span className="chosen-label">Countries</span>{countries.map(code=>
     <button key={code} type="button" onClick={()=>setCountries(current=>current.filter(entry=>entry!==code))} title={`Stop showing ${countryName(code)}`}>{countryName(code)} <span className="x">×</span></button>)}
     {!!unplaced&&<label className="inline" title="Some boards publish a count instead of a place and link to no office either, so nothing says which country they are in."><input type="checkbox" checked={includeUnknownLocations} onChange={e=>setIncludeUnknownLocations(e.target.checked)}/> Include {unplaced} with no known country</label>}</div>}
   </section>
  </div>
  <div className="scroll">
   {status&&<output className="status">{status}</output>}
   {!followed.length?<div className="empty"><strong>No sources are switched on</strong>
     <span>Switching a source off on the Sources tab retires it. Anything you already saved stays on the Applications tab.</span></div>
    :!selected.length?<div className="empty"><strong>No sources selected</strong>
     <span>Nothing is listed until at least one source is ticked above. This only changes what you are looking at — every source stays switched on.</span>
     <button onClick={()=>setShown(followed.map(s=>s.id))}>Show all sources</button></div>
    :isLoading?<div className="skeletons" aria-hidden="true">{["58%","71%","46%","64%","52%"].map(width=>
      <div key={width}><span style={{width}}/><span/><span/><span/></div>)}</div>
    :!jobs.length?(filtered
     ?<div className="empty"><strong>No job matches these filters</strong>
       <span>Clear them to see everything from the selected sources.</span>
       <button onClick={clearFilters}>Clear filters</button></div>
     :<div className="empty"><strong>No jobs stored for these sources yet</strong>
       <span>JobScraper is checking these sources automatically. New listings will appear here.</span></div>)
    // The header carries the same column widths as every row, so the list reads as one table
    // and nothing shifts when the next page arrives.
    :<><div className="list-head" aria-hidden="true"><button>Role</button><button>Company</button><span className="hide-md">Location</span><button className="hide-sm" onClick={()=>setSort(current=>current==="posted"?"recent":"posted")}>Posted <span className="arrow">{sort==="posted"?"▾":""}</span></button><span className="only-wide">Salary</span><span className="hide-sm">Status</span><span>Actions</span></div>
     <div className="jobs">{jobs.map((job,index)=><JobCard job={job} sourceName={names.get(job.sourceId)} onStatus={setStatus} onOpen={()=>setReaderIndex(index)} key={job.id}/>)}</div>
     {hasNextPage&&<div className="load-more"><button disabled={isFetchingNextPage} onClick={()=>fetchNextPage()}>{isFetchingNextPage?"Loading…":`Load 200 more`}</button></div>}</>}
  </div>
  {readerIndex!==null&&jobs[readerIndex]&&
   <JobReader jobs={jobs} index={readerIndex} sourceName={job=>names.get(job.sourceId)}
    onMove={setReaderIndex} onClose={()=>setReaderIndex(null)} onStatus={setStatus}/>
  }
 </div>}

// ---------------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------------
const logTone=(event:string)=>event==="failed"?"error":event==="warning"||event==="needs_user_action"?"warning":event==="completed"?"ok":"dim";
function SessionBadge({sourceId}:{sourceId:string}){const {data:saved}=useQuery({queryKey:["session",sourceId],queryFn:()=>hasBrowserSession(sourceId)});return saved?<span className="tag-session">SESSION</span>:null}
function Sources(){
 const qc=useQueryClient(); const {data:sources=[],isLoading,error}=useQuery({queryKey:["sources"],queryFn:api.listSources});
 const [selected,setSelected]=useState<Source|undefined>(); const [result,setResult]=useState(""); const [notices,setNotices]=useState<string[]>([]); const [paused,setPaused]=useState<WorkerEvent>(); const [preview,setPreview]=useState<ReturnType<typeof previewFromEvents>>();
 const [log,setLog]=useState<{id:number;runId:string;event:string;t:string;m:string;tone:string}[]>([]); const [lastRun,setLastRun]=useState<string>();const [rebuilding,setRebuilding]=useState(false);const [logShown,setLogShown]=useState(true);
 const runStarts=useRef(new Map<string,number>()),nextLogId=useRef(0);
 // Worker warnings are the only way the user learns that a site changed the terms — a redirect to
 // its own default location filter, a robots override, a host that moved. They are logged either
 // way, but the one that matters while adding a source has to be visible here and then.
  useEffect(()=>listenForScrapeEvents(listen,item=>{
   const received=Date.now();
   if(item.event==="started")runStarts.current.set(item.runId,received);
   const started=runStarts.current.get(item.runId),shown=started?{...item,payload:{...item.payload,elapsedMs:received-started}}:item;
   if(item.event==="started")setNotices([]);
  if(item.event==="warning"&&typeof item.payload.message==="string"){const text=item.payload.message;setNotices(current=>current.includes(text)?current:[...current,text])}
  if(item.event==="needs_user_action")setPaused(item);
   if(["completed","cancelled","failed"].includes(item.event))setPaused(current=>current?.runId===item.runId?undefined:current);
   if(item.event==="completed"){qc.invalidateQueries({queryKey:["sources"]});qc.invalidateQueries({queryKey:["jobs"]})}
   setLastRun(item.runId);
   setLog(current=>{
    const withoutProgress=item.event==="progress"||["completed","cancelled","failed"].includes(item.event)
     ?current.filter(entry=>entry.runId!==item.runId||entry.event!=="progress"):current;
    return [...withoutProgress,{id:nextLogId.current++,runId:item.runId,event:item.event,t:new Date(received).toTimeString().slice(0,8),m:describeLogEvent(shown),tone:logTone(item.event)}].slice(-200)});
   if(["completed","cancelled","failed"].includes(item.event))runStarts.current.delete(item.runId)}),[qc]);
 const save=useMutation({mutationFn:api.saveSource,onSuccess:()=>{qc.invalidateQueries({queryKey:["sources"]});setSelected(undefined)},onError:e=>setResult(String(e))});
 // Save closes the dialog first and reads the site afterwards. The probe can take tens of seconds
 // and used to hold the form open with a dead "Reading the site…" button; the wait belongs here,
 // on the page, where the status line and the run log both show what is happening.
 const submit=async(draft:SourceDraft)=>{setSelected(undefined);
  if(draft.manual)return save.mutate(sourceInput(draft,draft.adapterId,draft.config,draft.baseUrl,draft.name,draft.robots));
  setResult(`Reading ${draft.baseUrl}…`);
  try{const{payload,override,adapterId,found}=await configureSource(draft);
   const merged={...draft.config,...(payload.detectedConfig||{})},finalUrl=String(payload.finalUrl||draft.baseUrl),label=draft.name||String(payload.suggestedName||"")||finalUrl;
   const reason=found?null:describeUnsupported("no_listings");
   save.mutate(sourceInput(draft,adapterId,merged,finalUrl,label,override,reason));
   setResult(reason?`${label}: saved but left off — ${reason}`:`${label}: set up as ${adapterId}, ${found} listing${found===1?"":"s"} found.`)}
  catch(e){setResult(e instanceof Error?e.message:String(e))}};
 const test=async(source:Source)=>{setResult(`Testing ${source.name}…`);try{const events=await api.test(source);setPreview(previewFromEvents(events));setResult(describeOutcome(events,source.name))}catch(e){setResult(`${source.name}: ${describeFailure()} (see Diagnostics for details)`);console.error(e)}};
 const rebuild=()=>{const runId=crypto.randomUUID(),started=Date.now();setRebuilding(true);setResult("Rebuilding listings from every selected source in parallel…");scrapeAll(runId,[],true)
  .then(summary=>{setResult(`${describeUpdateSummary(summary)} ${elapsedSince(started)}. Descriptions still download only when opened.`);qc.invalidateQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["sources"]})})
  .catch(error=>setResult(String(error))).finally(()=>setRebuilding(false))};
 const toggle=(source:Source,enabled:boolean)=>api.setSourceEnabled(source.id,enabled).then(()=>{qc.invalidateQueries({queryKey:["sources"]});qc.invalidateQueries({queryKey:["jobs"]})}).catch(e=>setResult(String(e)));
 const lastRead=(source:Source)=>source.lastSuccessAt?`${new Date(source.lastSuccessAt).toLocaleString([],{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})} · ${source.jobCount.toLocaleString()} listings`:source.disabledReason||"Not read yet";
 return <div className={logShown?"sources-layout":"sources-layout log-hidden"}>
  <section className="sources-main">
   <header><div><h2>Sources</h2><p>Every selected source is read automatically when the app opens.</p></div>
    <div className="actions">{!logShown&&<button onClick={()=>setLogShown(true)}>Show run log</button>}<button disabled={rebuilding} onClick={rebuild}>{rebuilding?"Re-scrubbing…":"Re-scrub all"}</button>
     <button className="primary" onClick={()=>setSelected({id:"",name:"",baseUrl:"https://",adapterId:"static-css",adapterVersion:"1.1.0",enabled:false,kind:"active",robotsOverride:true,jobCount:0,closedCount:0})}>Add source</button></div></header>
   <div className="scroll">
    {paused&&<aside className="paused" aria-live="polite"><span className="tag">PAUSED</span>
     <span className="text">Login or CAPTCHA needs your action in headed Edge.</span>
     <button onClick={()=>cancelScrape(paused.runId)}>Cancel</button>
     <button className="primary" onClick={()=>api.resumeScrape(paused.runId)}>Resume &amp; save session</button></aside>}
    {result&&<output className="status">{result}</output>}
    {notices.map(text=><output className="status warning" key={text}>{text}</output>)}
    {isLoading?<p className="muted">Loading…</p>:<div className="source-table">
     <div className="source-head"><span>Source</span><span className="adapter">Adapter</span><span className="num">Jobs</span><span>Last read</span><span>On</span><span>Actions</span></div>
     {sources.map(s=><article className={s.disabledReason&&!s.enabled?"source-row attention":"source-row"} key={s.id}>
      <div><div className="source-name"><strong>{s.name}</strong>
        {supportsSessionCapture(s.adapterId)&&s.kind!=="reference"&&<SessionBadge sourceId={s.id}/>}
        {s.kind==="reference"&&<span className="pill">Reference</span>}</div>
       <span className="source-url">{s.baseUrl}</span>
       </div>
      <span className="source-adapter">{s.adapterId}</span>
      <span className="source-jobs"><strong>{s.jobCount.toLocaleString()}</strong>
       {s.closedCount>0&&<small title="Read from this source before, and no longer listed on it">{s.closedCount.toLocaleString()} closed</small>}</span>
      <span className="source-health">{s.disabledReason&&!s.enabled?<span className="mark-bad">!</span>:s.lastSuccessAt?<span className="mark-ok">✓</span>:null}<span>{lastRead(s)}</span></span>
      <span><input className="switch" type="checkbox" aria-label={`Select ${s.name}`} checked={s.enabled} disabled={s.kind==="reference"} onChange={e=>toggle(s,e.target.checked)}/></span>
      <div className="actions">
       <button onClick={()=>test(s)}>Test</button>
       <button onClick={()=>setSelected(s)}>Edit</button>
       <button className="danger" onClick={()=>confirm(`Remove ${s.name}? Jobs already saved to Applications are kept.`)&&api.deleteSource(s.id).then(()=>{qc.invalidateQueries({queryKey:["sources"]});qc.invalidateQueries({queryKey:["jobs"]})})}>Delete</button>
      </div></article>)}</div>}
    {error&&<p className="error">{String(error)}</p>}
    {preview&&<><div className="rule"><h3>Test preview · not saved</h3><hr/>
      <span className="count">{preview.jobs.length} row{preview.jobs.length===1?"":"s"} · {preview.mode} · {preview.complete?"complete":"partial"}</span>
      <button onClick={()=>setPreview(undefined)}>Close preview</button></div>
     <section className="panel" aria-label="Test preview">
      {preview.warnings.length>0&&<div className="panel-note">{preview.warnings.join(" · ")}</div>}
      <table><thead><tr><th>Title</th><th>Company</th><th>Location</th><th>Date</th><th>URL</th></tr></thead>
       <tbody>{preview.jobs.map((job,index)=><tr key={index}><td>{String(job.title??"—")}</td><td>{String(job.company??"—")}</td><td>{String(job.location??"—")}</td><td>{String(job.postedAt??"—")}</td><td>{job.applyUrl||job.canonicalUrl?<a href={String(job.applyUrl??job.canonicalUrl)} target="_blank" rel="noreferrer">Open</a>:"—"}</td></tr>)}</tbody></table>
      </section></>}
    <p className="hint" style={{marginTop:22}}>What each source stores is set once, in Settings → Collecting. The filters on the Jobs page only change what you see.</p>
   </div>
  </section>
  {logShown&&<aside className="runlog">
   <header><strong>Run log</strong><span className="caption">this session</span><button className="link close" onClick={()=>setLogShown(false)}>Hide</button></header>
   <div className="lines">{log.length?log.map(line=><div key={line.id}><time>{line.t}</time><span className={line.tone}>{line.m}</span></div>)
    :<span className="dim">Nothing yet. Tests and automatic updates appear here.</span>}</div>
   <footer><button disabled={!log.length} onClick={()=>navigator.clipboard?.writeText(log.map(line=>`${line.t} ${line.m}`).join("\n"))}>Copy log</button>
    <button className="danger" disabled={!lastRun} onClick={()=>lastRun&&cancelScrape(lastRun)}>Cancel run</button></footer>
  </aside>}
  {selected&&<SourceForm source={selected} onCancel={()=>setSelected(undefined)} onSave={submit}/>}
 </div>}
// Asked once, ever: after the first yes every blocked source overrides silently.
const ROBOTS_ACK="robots.autoAcknowledged";
// Layer one of the two filter layers. This one decides what is ever stored; the Jobs page
// boxes decide what is shown of what was stored. Mirrors db.rs::SCRAPE_TITLE_FILTER.
const SCRAPE_TITLE_FILTER="scrape.titleAny";
const configFields:Record<string,string[]>={"static-css":["itemSelector","titleSelector","companySelector","locationSelector","dateSelector","urlSelector","descriptionSelector","nextSelector","detailUrlField"],"static-xpath":["itemXPath","titleXPath","companyXPath","locationXPath","urlXPath","descriptionXPath","nextXPath"],playwright:["urlTemplate","itemSelector","titleSelector","nextSelector"],json:["urlTemplate","itemsPath","pageParam","detailUrlField"],rss:["urlTemplate"],apple:["locale","query"],workday:["tenant","site","listingPath","query","pageSize","detailUrlField","splitFacet","splitThreshold"],eightfold:["domain","eightfoldApi","query","pageSize"],icims:["listingPath","query","pageSize","detailUrlField"],"talentbrew-jibe":["listingPath","query","pageSize","detailUrlField"],phenom:["listingPath","query","pageSize","detailUrlField"],
 // Company boards derive their own URLs, so the only thing there is to set is how far to read.
 arm:["maxPages"],amd:["maxPages"],mediatek:["maxPages"],google:["maxPages"],cisco:["maxPages"],"sk-hynix":[],"u-blox":[]};
// Mirrors requiredFor() in sidecar/worker.mjs: fields an adapter cannot build a request without.
const requiredFields=(id:string)=>id==="static-css"?["itemSelector","titleSelector"]:id==="static-xpath"?["itemXPath","titleXPath"]:id==="json"?["itemsPath"]:id==="workday"?["tenant","site"]:id==="eightfold"?["domain"]:[];
// What the form collects. Reading the site is deliberately NOT part of it: the probe can take
// tens of seconds, and holding the dialog open for it made Save feel like it had hung.
export type SourceDraft={id?:string;kind:string;name:string;baseUrl:string;adapterId:string;config:Record<string,unknown>;enabled:boolean;robots:boolean;local:boolean;headless:boolean;manual:boolean};
const probeSource=(draft:SourceDraft,override:boolean)=>api.probe({id:draft.id??"",name:draft.name||"Probe",baseUrl:draft.baseUrl,adapterId:draft.adapterId||"static-css",adapterVersion:"1.1.0",enabled:false,kind:draft.kind,robotsOverride:override} as Source)
 .then(events=>{const final=events.at(-1);if(!final||final.event!=="completed")throw Error(describeFailure(final?.payload?.code as string|undefined));return final.payload as Record<string,any>});
// Probe identifies the platform AND, for plain HTML boards, infers selectors and proves them by
// parsing, so a source that saves is a source that already produced listings. Nothing here needs
// the user to know what an adapter is — and now nothing here keeps them waiting on a dialog.
const configureSource=async(draft:SourceDraft)=>{let payload=await probeSource(draft,draft.robots),override=draft.robots;
 if(payload.robotsBlocked&&!override){const acknowledged=await getSetting(ROBOTS_ACK).catch(()=>null);
  // window.confirm is not reliably honoured inside WebView2; the dialog plugin always is.
  if(acknowledged!=="true"){if(!await ask("This site asks automated tools not to read its job listings.\n\nScrape it anyway? This choice applies to every source you add.",{title:"Scrape this site?",kind:"warning"}))throw Error(describeFailure("robots_denied"));
   await setSetting(ROBOTS_ACK,"true")}
  override=true;payload=await probeSource(draft,true)}
 const verdict=probeVerdict(payload);
 if(!verdict.ok)throw Error(verdict.error);
 return{payload,override,adapterId:verdict.adapterId,found:verdict.found};};
const sourceInput=(draft:SourceDraft,adapterId:string,merged:Record<string,unknown>,finalUrl:string,label:string,override:boolean,disabledReason:string|null=null)=>
 ({id:draft.id||undefined,name:label||finalUrl,baseUrl:finalUrl,adapterId,kind:draft.kind,enabled:disabledReason?false:draft.enabled,robotsOverride:override,allowPrivateNetwork:draft.local,configJson:{...merged,headless:draft.headless,mode:merged.mode==="playwright"||adapterId==="playwright"?"playwright":"direct"},disabledReason});
// Saved once and applied to every source on its next automatic update. Empty means store everything:
// a filter that silently discarded a whole run would be indistinguishable from a broken scraper.
// What gets stored at all, as opposed to what the Jobs page chooses to show.
function ScrapeFilter(){
 const [title,setTitle]=useState("");const [saved,setSaved]=useState("");const [status,setStatus]=useState("");
 useEffect(()=>{getSetting(SCRAPE_TITLE_FILTER).then(stored=>{setTitle(stored??"");setSaved(stored??"")}).catch(()=>{})},[]);
 const save=()=>setSetting(SCRAPE_TITLE_FILTER,title).then(()=>{setSaved(title);
  setStatus(title.trim()?`Saved. Updates will store only titles containing ${title.trim()}.`:"Saved. Updates will store every listing.")})
  .catch(e=>setStatus(e instanceof Error?e.message:String(e)));
 return <div className="setting scrape-filter">
  <label><span>Title contains</span>
   <input value={title} onChange={e=>setTitle(e.target.value)} placeholder="verification, RTL, design verification"
    title="Comma-separated; a term can be a phrase. Empty keeps everything. Applied while scraping, so what it drops is never stored — the Jobs filters only change what you see."/></label>
  <small>Comma-separated; a term can be a phrase. Empty stores everything. This decides what is saved at all — Jobs filters only change what you see.</small>
  <div className="row"><button className="primary small" disabled={title===saved} onClick={save}>Save filter</button></div>
  {status&&<output className="status">{status}</output>}
 </div>}
function Settings({open,onClose}:{open:boolean;onClose:()=>void}){
 const qc=useQueryClient();const {data}=useQuery({queryKey:["diagnostics"],queryFn:api.diagnostics,enabled:open});
 const {data:background,error:backgroundError,refetch:refetchBackground}=useQuery({queryKey:["background-sync"],queryFn:backgroundSyncStatus,enabled:open});
 const {data:launchAtLogin,error:launchError,refetch:refetchLaunchAtLogin}=useQuery({queryKey:["launch-at-login"],queryFn:launchAtLoginStatus,enabled:open});
 const [message,setMessage]=useState("");const [backgroundBusy,setBackgroundBusy]=useState(false);const [launchBusy,setLaunchBusy]=useState(false);
 const [purging,setPurging]=useState(false);const [confirmation,setConfirmation]=useState("");const [purgeBusy,setPurgeBusy]=useState(false);
 const toggleBackground=(enabled:boolean)=>{setBackgroundBusy(true);setBackgroundSync(enabled).then(()=>{setMessage(enabled?"Background checks enabled.":"Background checks disabled.");return refetchBackground()}).catch(error=>setMessage(String(error))).finally(()=>setBackgroundBusy(false))};
 const toggleLaunch=(enabled:boolean)=>{setLaunchBusy(true);setLaunchAtLogin(enabled).then(()=>{setMessage(enabled?"Start at sign-in enabled.":"Start at sign-in disabled.");return refetchLaunchAtLogin()}).catch(error=>setMessage(String(error))).finally(()=>setLaunchBusy(false))};
 const closePurge=()=>{setPurging(false);setConfirmation("")};
 const purge=()=>{setPurgeBusy(true);purgeAllJobs(confirmation).then(result=>{setMessage(`Deleted ${result.deleted.toLocaleString()} stored job${result.deleted===1?"":"s"}${result.kept?`; kept ${result.kept.toLocaleString()} with an application or review`:""}.`);closePurge();for(const key of ["diagnostics","jobs","sources","apps"])qc.invalidateQueries({queryKey:[key]})}).catch(error=>setMessage(String(error))).finally(()=>setPurgeBusy(false))};
 if(!open)return null;
 return <><div className="scrim" onClick={onClose}/><aside className="panel-sheet" role="dialog" aria-modal="true" aria-label="Settings"><header><h2>Settings</h2><button className="link close" onClick={onClose}>Close</button></header>
  <div className="panel-body">{message&&<output className="status">{message}</output>}
   <section><span className="caption">Startup</span>
    <div className="setting"><label className="inline"><input type="checkbox" checked={launchAtLogin?.enabled??false} disabled={!launchAtLogin||launchBusy} onChange={event=>toggleLaunch(event.target.checked)}/> Open JobScraper when I sign in</label><small>Windows opens the full app after you sign in.{launchAtLogin?.debugBuild?" This development build registers its debug executable.":""}</small>{launchError&&<small className="warning">Windows Task Scheduler did not answer: {String(launchError)}</small>}</div>
    <div className="setting"><label className="inline"><input type="checkbox" checked={background?.enabled??false} disabled={!background||backgroundBusy} onChange={event=>toggleBackground(event.target.checked)}/> Check for new jobs in the background</label><small>Runs hidden at sign-in and every 4 hours, then shows one notification when new listings appear.{background?.debugBuild?" This development build uses the JobScraper-dev database.":""}</small>{backgroundError&&<small className="warning">Windows Task Scheduler did not answer: {String(backgroundError)}</small>}</div>
   </section>
   <section><span className="caption">Collecting</span><ScrapeFilter/></section>
   <section><span className="caption">Reminders</span><div className="setting"><small>Rebuild pending application and interview reminders after changing system tasks or restoring data.</small><div className="row"><button onClick={()=>api.reconcileReminders().then(result=>setMessage(JSON.stringify(result))).catch(error=>setMessage(String(error)))}>Reconcile reminders</button></div></div></section>
   <section><span className="caption">Data</span><div className="setting"><small>{data?`${data.dbPath} · ${data.jobCount.toLocaleString()} listings · schema ${data.schemaVersion}`:"Loading local data…"}</small></div>
    <div className="danger-zone"><strong>Delete all stored jobs</strong><small>Sources and their settings stay, and anything saved or applied to is kept. The next update re-reads every source in full.</small><button className="small danger" onClick={()=>setPurging(true)}>Delete all jobs…</button></div></section>
  </div></aside>
  {purging&&<dialog open className="confirm" aria-label="Delete every stored job"><div className="sheet"><div className="dialog-body"><h2>Delete every stored job?</h2><p>This removes all {data?.jobCount?.toLocaleString()??""} stored listings and their sighting history. Sources, settings, and saved applications stay.</p><p className="detail">This cannot be undone.</p><label><span>Type Confirm to continue</span><input autoFocus value={confirmation} onChange={event=>setConfirmation(event.target.value)} placeholder="Confirm"/></label></div><footer><button className="link" onClick={closePurge}>Cancel</button><button className="danger" disabled={purgeBusy||confirmation.trim()!=="Confirm"} onClick={purge}>{purgeBusy?"Deleting…":"Delete all jobs"}</button></footer></div></dialog>}
 </>}
function SourceForm({source,onCancel,onSave}:{source:Source;onCancel:()=>void;onSave:(draft:SourceDraft)=>void}){
 const qc=useQueryClient();const canCapture=Boolean(source.id&&source.kind!=="reference"&&supportsSessionCapture(source.adapterId));
 const {data:savedSession}=useQuery({queryKey:["session",source.id],queryFn:()=>hasBrowserSession(source.id),enabled:canCapture});
 const [url,setUrl]=useState(source.baseUrl==="https://"?"":source.baseUrl),[name,setName]=useState(source.name),[adapter,setAdapter]=useState(source.id?source.adapterId:""),[config,setConfig]=useState<Record<string,unknown>>({pageSize:20,maxPages:50});
 const [enabled,setEnabled]=useState(source.id?source.enabled:true),[robots,setRobots]=useState(source.id?source.robotsOverride:true),[local,setLocal]=useState(false),[headless,setHeadless]=useState(true),[adjust,setAdjust]=useState(false),[error,setError]=useState(""),[sessionStatus,setSessionStatus]=useState("");
 useEffect(()=>{if(source.id)sourceConfig(source.id).then(setConfig).catch(()=>{})},[source.id]);
 const missing=(requiredFields(adapter)).filter(key=>!String(config[key]||"").trim());
 // An adapter chosen by hand under Adjust is taken as-is; everything else configures itself.
 const save=(event:React.FormEvent)=>{event.preventDefault();
  if(!(url.startsWith("http://")||url.startsWith("https://")))return setError("Enter the full web address of the jobs page.");
  const manual=adjust&&!!adapter;
  if(manual&&missing.length)return setError(`${adapter} needs: ${missing.join(", ")}.`);
  setError("");
  onSave({id:source.id||undefined,kind:source.kind,name,baseUrl:url,adapterId:adapter,config,enabled,robots,local,headless,manual})};
 const capture=async()=>{setSessionStatus("Opening a browser session…");try{const events=await captureSession(source);setSessionStatus(events.at(-1)?.event==="completed"?"Encrypted session saved. Cookie values are never displayed.":describeOutcome(events,source.name));qc.invalidateQueries({queryKey:["session",source.id]})}catch(error){setSessionStatus(String(error))}};
 const forget=()=>confirm("Delete the encrypted saved session for this source?")&&deleteBrowserSession(source.id).then(()=>{setSessionStatus("Saved session deleted.");qc.invalidateQueries({queryKey:["session",source.id]})});
 return <dialog open><form onSubmit={save}>
  <header><h3>{source.id?"Edit":"Add"} source</h3></header>
  <div className="dialog-body">
   <label><span>Jobs page URL</span><input autoFocus value={url} onChange={e=>setUrl(e.target.value)} placeholder="https://company.com/careers"
    title="Paste the careers page address and press Save. This closes straight away; the site is read in the background and the Sources page says how it went — you never need to know what an adapter is."/></label>
   <label><span>Name</span><input value={name} onChange={e=>setName(e.target.value)} placeholder="Filled in automatically"
    title="Left empty, the site's own name is used once it has been read."/></label>
   <button type="button" className="link" onClick={()=>setAdjust(!adjust)}>{adjust?"Hide":"Show"} advanced settings</button>
   {adjust&&<section className="advanced"><div className="field-grid">
    <label><span>Adapter</span><select value={adapter} onChange={e=>{setAdapter(e.target.value);setConfig({pageSize:20,maxPages:50})}}><option value="">Not set</option>{Object.keys(configFields).map(id=><option key={id}>{id}</option>)}</select></label>
    <label><span>Max pages</span><input type="number" min="1" max="500" value={Number(config.maxPages||50)} onChange={e=>setConfig(c=>({...c,maxPages:Number(e.target.value)}))}/></label>
    {(configFields[adapter]||[]).map(key=><label key={key}><span>{key}{requiredFields(adapter).includes(key)?" *":""}</span><input value={String(config[key]||"")} onChange={e=>setConfig(current=>({...current,[key]:key==="pageSize"?Number(e.target.value):e.target.value}))}/></label>)}</div>
    <hr/>
    <label className="inline"><input type="checkbox" checked={headless} onChange={e=>setHeadless(e.target.checked)}/> Headless Edge</label>
    <label className="inline"><input type="checkbox" checked={robots} onChange={e=>setRobots(e.target.checked)}/> Override robots.txt</label>
    <label className="inline"><input type="checkbox" checked={local} onChange={e=>setLocal(e.target.checked)}/> Allow private/local network</label>
   <label className="inline"><input type="checkbox" checked={enabled} onChange={e=>setEnabled(e.target.checked)}/> Select this source</label></section>}
   {canCapture&&<section className="advanced"><div className="actions"><span className="hint">Browser session: {savedSession?"saved":"none"}</span><button type="button" onClick={capture}>Capture session</button>{savedSession&&<button type="button" className="danger" onClick={forget}>Forget session</button>}</div>{sessionStatus&&<output className="status">{sessionStatus}</output>}</section>}
   {source.kind==="reference"&&<p className="warning">Reference source never scrapes.</p>}{error&&<p className="error">{error}</p>}
  </div>
  <footer><button type="button" onClick={onCancel}>Cancel</button>
   <button className="primary" disabled={!url}>Save</button></footer></form></dialog>
}

// ---------------------------------------------------------------------------------------------
// Applications
// ---------------------------------------------------------------------------------------------
function localDateTimeValue(utc:string){const date=new Date(utc);const offset=date.getTimezoneOffset()*60_000;return new Date(date.getTime()-offset).toISOString().slice(0,16)}
export function InterviewPanel({applicationId,refresh}:{applicationId:string;refresh:()=>void}){const [open,setOpen]=useState(false);const [opened,setOpened]=useState(false);const {data:items=[],refetch}=useQuery({queryKey:["interviews",applicationId],queryFn:()=>api.interviews(applicationId),enabled:open,staleTime:Infinity});const [stage,setStage]=useState("phone screen");const [when,setWhen]=useState("");const [notes,setNotes]=useState("");const [outcome,setOutcome]=useState("");const [editing,setEditing]=useState<string>();const [message,setMessage]=useState("");const save=()=>{if(!when)return;api.saveInterview({id:editing,applicationId,stage,scheduledAt:new Date(when).toISOString(),notes:notes||null,outcome:outcome||null}).then(()=>{setMessage(editing?"Interview and reminders rescheduled.":"Interview reminders scheduled for 24h and 1h before.");setWhen("");setNotes("");setOutcome("");setEditing(undefined);refetch();refresh()}).catch(e=>setMessage(String(e)))};return <details onToggle={event=>{const next=event.currentTarget.open;setOpen(next);if(next)setOpened(true)}}><summary>Interviews &amp; reminders</summary>{opened&&<><label>Stage<select value={stage} onChange={e=>setStage(e.target.value)}><option>phone screen</option><option>recruiter</option><option>technical</option><option>onsite</option><option>final</option><option>custom</option></select></label><label>Local date/time<input type="datetime-local" value={when} onChange={e=>setWhen(e.target.value)}/></label><label>Notes<input value={notes} onChange={e=>setNotes(e.target.value)}/></label><label>Outcome<input value={outcome} onChange={e=>setOutcome(e.target.value)}/></label><button type="button" onClick={save}>{editing?"Reschedule interview":"Schedule interview"}</button>{items.map(item=><div key={item.id}><span>{item.stage} · {new Date(item.scheduledAt).toLocaleString()} {item.outcome&&`· ${item.outcome}`}</span><button type="button" onClick={()=>{setEditing(item.id);setStage(item.stage);setWhen(localDateTimeValue(item.scheduledAt));setNotes(item.notes||"");setOutcome(item.outcome||"")}}>Edit</button><button type="button" onClick={()=>api.deleteInterview(item.id).then(()=>refetch())}>Cancel</button></div>)}<label>Ghost threshold days<input type="number" defaultValue={14} min="1" max="365" onBlur={e=>api.ghostThreshold(applicationId,Number(e.target.value)).then(()=>setMessage("Ghost reminder rescheduled.")).catch(err=>setMessage(String(err)))}/></label>{message&&<small>{message}</small>}</>}</details>}
function ApplicationDrawer({applicationId,onClose,refresh}:{applicationId:string;onClose:()=>void;refresh:()=>void}){
 const {data,refetch:queryRefetch}=useQuery({queryKey:["application-details",applicationId],queryFn:()=>api.applicationDetails(applicationId)});const refetch=()=>queryRefetch();const [note,setNote]=useState("");const [message,setMessage]=useState("");if(!data)return <dialog open className="drawer" aria-label="Application details"><div className="sheet"><div className="dialog-body"><p className="muted">Loading details…</p></div></div></dialog>;const app=data.application;
 const saveDetails=(event:React.FormEvent<HTMLFormElement>)=>{event.preventDefault();const form=new FormData(event.currentTarget);api.saveApplicationDetails({applicationId,recruiterName:form.get("name")||null,recruiterEmail:form.get("email")||null,recruiterPhone:form.get("phone")||null,sourceAttribution:form.get("source")||null,rejectionReason:form.get("rejection")||null,rejectionCategory:form.get("category")||null,withdrawnReason:form.get("withdrawn")||null}).then(()=>{setMessage("Details saved.");refetch();refresh()}).catch(e=>setMessage(String(e)))};const attachFile=(file:File,type:string)=>{try{validateAttachmentSize(file.size)}catch(error){setMessage(String(error));return}file.arrayBuffer().then(bytes=>api.attachDocument({applicationId,documentType:type,filename:file.name,mimeType:file.type||"application/octet-stream"},bytes)).then(()=>{setMessage("Immutable snapshot attached.");refetch()}).catch(e=>setMessage(String(e)))};const exportFile=(item:typeof data.documents[number])=>api.exportDocument(item.id).then(bytes=>{const url=URL.createObjectURL(new Blob([bytes],{type:item.mimeType}));const link=document.createElement("a");link.href=url;link.download=item.filename;link.click();URL.revokeObjectURL(url)}).catch(e=>setMessage(String(e)));
 const moveStage=(stage:string)=>{if(stage===app.currentStage)return;const error=boardMoveError(app.currentStage,stage);if(error)return setMessage(error);api.stage(app.id,stage).then(()=>{setMessage(`Moved to ${stageLabel(stage)}.`);refetch();refresh()}).catch(error=>setMessage(String(error)))};
 return <dialog open className="drawer" aria-label="Application details"><div className="sheet">
  <header><div><h2>{app.title||"Application"}</h2>
    <p>{app.company} · {stageLabel(app.currentStage)}{data.firstResponseAt?` · first response ${new Date(data.firstResponseAt).toLocaleString()}`:" · no response yet"}</p></div>
   <button className="close" aria-label="Close" onClick={onClose}>✕</button></header>
  <div className="dialog-body">
   {message&&<output className="status">{message}</output>}
   <p className="hint">{data.sourceName}{data.sourceUrl&&<> · <a href={data.sourceUrl} target="_blank" rel="noreferrer">Open source</a></>}</p>
   <label><span>Stage</span><select value={app.currentStage} onChange={event=>moveStage(event.target.value)}>{applicationStages.map(stage=><option key={stage} value={stage}>{stageLabel(stage)}</option>)}</select></label>
   <form onSubmit={saveDetails}>
    <span className="caption">Recruiter</span>
    <div className="field-grid">
     <label><span>Name</span><input name="name" defaultValue={app.recruiterName||""}/></label>
     <label><span>Email</span><input name="email" type="email" defaultValue={app.recruiterEmail||""}/></label>
     <label><span>Phone</span><input name="phone" defaultValue={app.recruiterPhone||""}/></label>
     <label><span>Source attribution</span><input name="source" defaultValue={app.sourceAttribution||""}/></label></div>
    <span className="caption">Outcome</span>
    <div className="field-grid">
     <label><span>Rejection reason</span><input name="rejection" defaultValue={app.rejectionReason||""}/></label>
     <label><span>Rejection category</span><input name="category" defaultValue={app.rejectionCategory||""}/></label>
     <label className="wide"><span>Withdrawal reason</span><input name="withdrawn" defaultValue={app.withdrawnReason||""}/></label></div>
    <button className="quiet">Save details</button></form>
   <hr/>
   <InterviewPanel applicationId={applicationId} refresh={()=>{refetch();refresh()}}/>
   <hr/>
   <section><span className="caption">Notes</span>
    <textarea rows={3} placeholder="Add a note…" value={note} onChange={e=>setNote(e.target.value)}/>
    <button className="quiet" disabled={!note.trim()} onClick={()=>api.saveNote({applicationId,body:note}).then(()=>{setNote("");refetch()}).catch(e=>setMessage(String(e)))}>Add note</button>
    {data.notes.map(item=><article className="note" key={item.id}><p>{item.body}</p>
     <div className="actions"><small>{new Date(item.updatedAt||item.createdAt).toLocaleString()}</small>
      <button className="link" onClick={()=>{const body=prompt("Edit note",item.body);if(body)api.saveNote({id:item.id,applicationId,body}).then(()=>refetch())}}>Edit</button>
      <button className="link" onClick={()=>api.deleteNote(item.id).then(()=>refetch())}>Delete</button></div></article>)}</section>
   <hr/>
   <section><span className="caption">Documents</span>
    {data.documents.map(doc=><div className="row" key={doc.id}><div><span>{doc.filename}</span>
      <small>{doc.documentType} · {doc.size} B · immutable snapshot</small></div>
      <button onClick={()=>exportFile(doc)}>Export</button></div>)}
    <label><span>Attach a local file</span><input type="file" onChange={e=>e.target.files?.[0]&&attachFile(e.target.files[0],"cover_letter")}/></label></section>
   <hr/>
   <section><span className="caption">Timeline</span>
    {data.events.map(event=><div className="event" key={event.id}><span className="dot"/>
     <div><span>{event.eventType}{event.fromStage?` · ${stageLabel(event.fromStage)} → ${stageLabel(event.toStage)}`:""}{event.reason?` · ${event.reason}`:""}</span>
      <small>{new Date(event.occurredAt).toLocaleString()}</small></div></div>)}</section>
  </div></div></dialog>}
function AppCard({app,details}:{app:Application;details:(id:string)=>void}){const drag=useDraggable({id:app.id,data:{application:app}});const[message,setMessage]=useState("");const when=app.appliedAt?`applied ${new Date(app.appliedAt).toLocaleDateString()}`:app.acceptedAt?`accepted ${new Date(app.acceptedAt).toLocaleDateString()}`:"saved";return <article className="card" ref={drag.setNodeRef} {...drag.listeners} {...drag.attributes} style={{opacity:drag.isDragging?.55:1}}><strong>{app.title||"Job"}</strong><p>{app.company}</p><span className="when">{when}</span><span className="card-actions"><button className="small" onPointerDown={e=>e.stopPropagation()} onClick={()=>details(app.id)}>Open</button>{app.currentStage==="planned"&&<button className="small primary" onPointerDown={e=>e.stopPropagation()} onClick={()=>api.openApply(app.id).then(()=>setMessage("Application opened. Confirm when JobScraper regains focus.")).catch(e=>setMessage(String(e)))}>Apply</button>}</span>{message&&<small>{message}</small>}</article>}
function StageColumn({stage,count,children}:{stage:string;count:number;children:React.ReactNode}){const drop=useDroppable({id:stage});
 const folded=["accepted","rejected","withdrawn"].includes(stage);return <section ref={drop.setNodeRef} className={[drop.isOver?"drop-active":"",folded?"folded":""].filter(Boolean).join(" ")}>
  <h3><span className="name">{stageLabel(stage)}</span><span className="count">{count}</span></h3>
  {count?children:<div className="empty">{stage==="planned"?"Nothing saved":"Nothing here"}</div>}</section>}
function Applications(){const qc=useQueryClient();const {data:apps=[]}=useQuery({queryKey:["apps"],queryFn:api.apps});const [selected,setSelected]=useState<string>();const [error,setError]=useState("");
 const refresh=()=>{qc.invalidateQueries({queryKey:["apps"]});qc.invalidateQueries({queryKey:["jobs"]})};
 const move=(a:Application,stage:string)=>{if(stage===a.currentStage)return;const err=boardMoveError(a.currentStage,stage);if(err){setError(err);return}api.stage(a.id,stage).then(refresh).catch(e=>setError(String(e)))};
 const onDragEnd=(event:DragEndEvent)=>{const app=event.active.data.current?.application as Application|undefined;const stage=event.over?.id;if(app&&typeof stage==="string")move(app,stage)};
 const openCount=apps.filter(app=>!["accepted","rejected","withdrawn"].includes(app.currentStage)).length,interviewing=apps.filter(app=>app.currentStage==="interviewing").length;
 return <div className="view">
  <div className="toolbar"><header><div><h2>Applications</h2><p>Drag a card between columns, or open it for notes, documents and interviews. Applied is recorded when you confirm after opening a posting.</p></div><span className="count">{openCount} open · {interviewing} interviewing</span></header>
   {error&&<output className="status warning">{error}</output>}</div>
  <div className="scroll" style={{display:"block"}}>
   {!apps.length?<div className="empty"><strong>Nothing saved yet</strong><span>Press Save or Apply on a job and it lands here as Saved.</span></div>
    :<DndContext onDragEnd={onDragEnd}><div className="board">{applicationStages.map(stage=>{const cards=apps.filter(a=>a.currentStage===stage);
     return <StageColumn stage={stage} count={cards.length} key={stage}>{cards.map(a=><AppCard app={a} details={setSelected} key={a.id}/>)}</StageColumn>})}</div></DndContext>}
  </div>
  {selected&&<ApplicationDrawer applicationId={selected} onClose={()=>setSelected(undefined)} refresh={refresh}/>}
 </div>}

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------
function Diagnostics(){const {data,refetch}=useQuery({queryKey:["diagnostics"],queryFn:api.diagnostics});const {data:restore}=useQuery({queryKey:["restore-status"],queryFn:api.restoreStatus});const {data:sources=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources});
 const attention=sources.filter(source=>source.kind!=="reference"&&!source.enabled&&source.disabledReason);
 const active=sources.filter(source=>source.kind!=="reference"&&source.enabled).length,total=sources.filter(source=>source.kind!=="reference").length;
 return <div className="view">
  <div className="toolbar"><header><div><h2>Diagnostics</h2><p>Local health information. Nothing here is sent anywhere.</p></div><div className="actions"><button onClick={()=>refetch()}>Refresh</button><button disabled={!data} onClick={()=>data&&navigator.clipboard?.writeText(JSON.stringify(data,null,2))}>Copy report</button></div></header></div>
  <div className="scroll">
   <div className="summary-row"><div><span>Database</span><strong>{data?`Schema ${data.schemaVersion}`:"Loading"}</strong><small>{data?.dbPath||"Opening local data"}</small></div><div className={attention.length?"bad":""}><span>Sources read</span><strong>{active} of {total}</strong><small>{attention.length?`${attention.length} need attention`:"No source failures"}</small></div><div><span>Listings stored</span><strong>{data?.jobCount.toLocaleString()??"—"}</strong><small>{data?`${data.sourceCount} sources in database`:"Loading"}</small></div><div><span>Sidecar</span><strong>{data?.sidecarActive?"Running":"Idle"}</strong><small>starts only on test or update</small></div>{restore&&<div><span>Restore</span><strong>{restore.applied?"Applied":"Ready"}</strong><small>{restore.message}</small></div>}</div>
   {!!attention.length&&<><div className="rule"><h3>Needs attention</h3><span className="count">{attention.length} source{attention.length===1?"":"s"}</span></div><div className="attn">{attention.map(source=><div key={source.id}><strong>{source.name}</strong><span className="why">{source.disabledReason}</span><span className="actions"><button className="small" onClick={()=>api.test(source)}>Test now</button></span></div>)}</div></>}
   <ScrapePerformance/>
   <ActivityLog/>
   <p className="hint">City and country data © <a href="https://www.geonames.org" target="_blank" rel="noreferrer">GeoNames</a>, CC BY 4.0. Bundled with the app; nothing is looked up online.</p>
   {data&&<details className="fold" style={{marginTop:34}}><summary>Raw report (JSON)</summary><pre>{JSON.stringify(data,null,2)}</pre></details>}
  </div>
 </div>}
// Where a run's time went. Only runs recorded since instrumentation shipped appear here; older
// log rows stay readable in Recent activity but carry no metrics to rank.
function ScrapePerformance(){const {data,refetch}=useQuery({queryKey:["scrape-performance"],queryFn:scrapePerformance});
 const latest=latestSummary(data),rows=aggregateRows(data),failures=recentFailures(data),run=data?.recent?.[0];
 const groups:[string,"worker"|"app"|"work"][]=[["Worker","worker"],["Critical path","app"],["App work","work"]];
 return <>
  <div className="rule"><h3>Scrape performance</h3><hr/><span className="count">last 50 measured runs</span>
   <button onClick={()=>refetch()}>Refresh</button>
   <button disabled={!data} onClick={()=>data&&navigator.clipboard?.writeText(JSON.stringify(data,null,2))}>Copy JSON</button></div>
  {!latest?<p className="empty"><strong>Nothing measured yet</strong><span>Updating or checking a source records where its time went.</span></p>
  :<>
   <div className="health">
    <div className="ok"><strong><span className="dot"/>Latest run</strong>
     <span className="detail">{`${latest.sourceName} · ${latest.action} · ${latest.outcome}\n${latest.total} at ${latest.at}`}</span></div>
    <div><strong><span className="dot"/>Slowest step</strong>
     <span className="detail">{latest.slowest?`${latest.slowest.label}\n${formatDuration(latest.slowest.milliseconds)}`:"not measured"}</span></div>
    <div><strong><span className="dot"/>Work done</strong>
     <span className="detail">{`${latest.requests} requests · ${latest.pages} pages\n${latest.jobs} jobs`}</span></div>
   </div>
   {rows.length>0&&<section className="panel log-table" aria-label="Scrape performance by source">
    <table><thead><tr><th>Source</th><th>Action</th><th>Runs</th><th>Median</th><th>p95</th><th>Slowest step</th></tr></thead><tbody>
     {rows.map(row=><tr key={`${row.label}-${row.action}`}><td>{row.label}</td><td>{row.action}</td><td>{row.samples}</td>
      <td>{row.median}</td><td>{row.p95??"—"}</td><td>{row.slowest??"—"}</td></tr>)}
    </tbody></table></section>}
   {run&&groups.map(([title,group])=>{const phases=phaseRows(run,group);return phases.length===0?null:
    <details key={group}><summary>{`${title} breakdown`}</summary><pre>{phases.map(phase=>`${phase.label}: ${formatDuration(phase.milliseconds)}`).join("\n")}</pre></details>})}
   {run&&requestKindRows(run).length>0&&<details><summary>Requests by kind</summary>
    <pre>{requestKindRows(run).map(kind=>`${kind.kind}: ${kind.count} · responses ${formatDuration(kind.responseMs)} · bodies ${formatDuration(kind.bodyMs)} · pacing ${formatDuration(kind.pacingMs)} · retry waits ${formatDuration(kind.backoffMs)}`).join("\n")}</pre></details>}
   {failures.length>0&&<details><summary>{`Recent failures (${failures.length})`}</summary>
    <pre>{failures.map(entry=>`${entry.at.replace("T"," ").slice(0,19)} · ${entry.sourceName??"all sources"} · ${entry.action} · ${entry.outcome} · ${formatDuration(entry.totalMs)}`).join("\n")}</pre></details>}
  </>}</>}
function ActivityLog(){const {data:logs=[],refetch}=useQuery({queryKey:["app-logs"],queryFn:()=>appLogs(200)});
 return <>
  <div className="rule"><h3>Recent activity</h3><hr/><span className="count">last 200 entries</span>
   <button onClick={()=>refetch()}>Refresh</button></div>
  {logs.length===0?<p className="empty"><strong>Nothing yet</strong><span>Testing and updating a source both record what happened here.</span></p>
  :<section className="panel log-table" aria-label="Recent activity"><table><thead><tr><th>When</th><th>Source</th><th>Action</th><th>What happened</th></tr></thead><tbody>
   {logs.map(entry=><tr key={entry.id}><td>{entry.at.replace("T"," ").slice(0,19)}</td><td>{entry.sourceName||"—"}</td><td>{entry.action.replace("_source","").replace("_"," ")}</td>
    <td className={entry.level==="error"?"error":entry.level==="warning"?"warning":undefined}>{entry.message}
     <details><summary>Details</summary><pre>{entry.detailJson}</pre></details></td></tr>)}
  </tbody></table></section>}</>}
const root=document.getElementById("root");
if(root)createRoot(root).render(<React.StrictMode><QueryClientProvider client={queryClient}><StartupGate><App/></StartupGate></QueryClientProvider></React.StrictMode>);

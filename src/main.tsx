import React, { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useInfiniteQuery, useIsFetching, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { DndContext, useDraggable, useDroppable, type DragEndEvent } from "@dnd-kit/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { aggregateRows, formatDuration, latestSummary, phaseRows, recentFailures, requestKindRows } from "./performance-ui";
import { COUNTRIES } from "./countries";
import { ACTIVE_FILTER_KEY, describePreset, parseActiveFilter, parsePresets, presetFrom, PRESETS_KEY, profileKey, readProfileSetting, serializeActiveFilter, withoutPreset, withPreset, type ActiveFilter, type FilterPreset } from "./presets";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyProfile, captureJob, countJobs, jobCountries, listCompanyCatalog, listProfiles, refreshJobs, REVIEWED_KEY, saveCapturedJob, setJobDismissed, type CapturedJob, type CatalogCompany } from "./api";
import { api, appLogs, backgroundSyncStatus, DEFAULT_POSTED_WINDOW, jobDescription, launchAtLoginStatus, POSTED_WINDOWS, purgeAllJobs, setBackgroundSync, setLaunchAtLogin, startupStatus, type StartupStatus, cancelScrape, cancelScrapeAll, captureSession, deleteBrowserSession, getSetting, setSetting, hasBrowserSession, scrapeAll, scrapePerformance, sourceConfig, startAutomaticSync, type Application, type Job, type JobDescription, type JobFilter, type Source, type WorkerEvent } from "./api";
import { listenForScrapeEvents } from "./scrape-events";
import { describeRow, feedProgressStore, isRunning, progressStore, rowFraction, useSourceProgress, type BatchProgress, type RowProgress } from "./source-progress";
import { catalogSource, describeFailure, duration, describeOutcome, describeProfileApplied, describeUnsupported, describeUpdateSummary, previewFromEvents, probeVerdict, supportsSessionCapture } from "./source-ui";
import { BOARD_COLUMNS, boardColumn, columnLabel, isApplied, isEnded, stageLabel, websiteUrl, type BoardColumn } from "./application-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./styles.css";

const queryClient=new QueryClient({defaultOptions:{queries:{refetchOnWindowFocus:false}}}); const pages=["Jobs","Sources","Applications","Diagnostics"] as const; type Page=typeof pages[number];
function useDebounced<T>(value:T,delay=250){const [settled,setSettled]=useState(value);useEffect(()=>{const timer=setTimeout(()=>setSettled(value),delay);return()=>clearTimeout(timer)},[value,delay]);return settled}
const elapsedSince=(started:number)=>{const seconds=Math.max(0,(Date.now()-started)/1000);return seconds<60?`${seconds.toFixed(1)}s`:`${Math.floor(seconds/60)}m ${Math.round(seconds%60)}s`};
const showSelect=(event:React.PointerEvent<HTMLSelectElement>)=>{if(event.button!==0||typeof event.currentTarget.showPicker!=="function")return;event.preventDefault();event.currentTarget.showPicker()};
const dragWindow=(event:React.MouseEvent<HTMLElement>)=>{if(event.button===0&&!(event.target as Element).closest("button,select,label"))void getCurrentWindow().startDragging()};
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
// "Updated 11:23" read as today even when the last update was days ago.
export function updatedLabel(at:number,now=Date.now()){
 const when=new Date(at),time=when.toLocaleTimeString([],{hour:"2-digit",minute:"2-digit"});
 const midnight=new Date(now).setHours(0,0,0,0);
 if(at>=midnight)return `Updated ${time}`;
 if(at>=midnight-86_400_000)return `Updated yesterday ${time}`;
 return `Updated ${when.toLocaleDateString([],{month:"short",day:"numeric"})} ${time}`;
}
function App(){
 const [page,setPage]=useState<Page>("Jobs");const [visited,setVisited]=useState<Page[]>(["Jobs"]);
 const [settingsOpen,setSettingsOpen]=useState(false);const [updateRun,setUpdateRun]=useState<string>();const [updateStatus,setUpdateStatus]=useState("");
 const updating=!!updateRun;
 // A message in the bar is news for a few seconds; then the bar goes back to saying when the last update was.
 useEffect(()=>{if(!updateStatus||updating)return;const timer=setTimeout(()=>setUpdateStatus(""),12_000);return()=>clearTimeout(timer)},[updateStatus,updating]);
 // The tab badges are the only place the three counts appear together. Sources and applications
 // come from the same cached queries the pages use; the job count is whatever Jobs is showing
 // after its filters, so it is reported upwards rather than re-queried unfiltered.
 const [jobCount,setJobCount]=useState<number>();
 const qc=useQueryClient();
 const {data:allSources=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources,refetchOnWindowFocus:true});
 const {data:allApps=[]}=useQuery({queryKey:["apps"],queryFn:api.apps,refetchOnWindowFocus:true});
 const activeSources=allSources.filter(source=>source.kind!=="reference"&&source.enabled).length;
 const scrapableSources=allSources.filter(source=>source.kind!=="reference").length;
 const attention=allSources.filter(source=>source.kind!=="reference"&&!source.enabled&&source.disabledReason).length;
 const badges:Record<Page,string>={Jobs:jobCount===undefined?"":String(jobCount),Sources:scrapableSources?`${activeSources} of ${scrapableSources}`:"0",Applications:String(allApps.length),Diagnostics:attention?String(attention):""};
 // A page is mounted the first time it is opened and only hidden afterwards, never unmounted.
 // Scraping always continued in the backend when the user switched away — what was thrown away
 // was every trace of it in the window: the Updating…/Cancel state of Update Jobs, the running
 // state of Update this source, the test preview, and the finished message. Leaving the page
 // mounted keeps all of it, so leaving and coming back shows the run exactly where it got to.
 // Per-source progress is collected from launch, before the Sources tab is ever opened.
 useEffect(()=>feedProgressStore(listen),[]);
 const open=(next:Page)=>{setPage(next);setVisited(current=>current.includes(next)?current:[...current,next])};
 // Other pages can send the user to Browse companies; the Sources page opens it when this number moves.
 const [browseRequest,setBrowseRequest]=useState(0);
 const browseCompanies=()=>{open("Sources");setBrowseRequest(count=>count+1)};
 const views:Record<Page,React.ReactNode>={Jobs:<Jobs onCount={setJobCount} onBrowse={browseCompanies} onOpenSources={()=>open("Sources")} onStatus={setUpdateStatus}/>,Sources:<Sources browseRequest={browseRequest} onStatus={setUpdateStatus}/>,Applications:<Applications/>,Diagnostics:<Diagnostics/>};
 const lastUpdate=allSources.map(source=>source.lastSuccessAt?new Date(source.lastSuccessAt).getTime():0).reduce((latest,value)=>Math.max(latest,value),0);
 const updateNow=()=>{const runId=crypto.randomUUID();setUpdateRun(runId);setUpdateStatus("Updating sources");scrapeAll(runId,[],false)
  .then(summary=>{setUpdateStatus(describeUpdateSummary(summary));refreshJobs(qc);qc.invalidateQueries({queryKey:["sources"]})})
  .catch(error=>setUpdateStatus(String(error))).finally(()=>setUpdateRun(undefined))};
 useEffect(()=>{const close=(event:KeyboardEvent)=>{if(event.key==="Escape")setSettingsOpen(false)};window.addEventListener("keydown",close);return()=>window.removeEventListener("keydown",close)},[]);
 const barText=updateStatus||(lastUpdate?`${updatedLabel(lastUpdate)}${attention?` · ${attention} source${attention===1?"":"s"} need attention`:""}`:"Not updated yet");
 return <main>
  <div className="titlebar" onMouseDown={dragWindow}><img className="mark" src="/logo.png" alt="" width={28} height={28} draggable={false}/><h1>JobScraper</h1><ProfileSwitcher onStatus={setUpdateStatus}/><div className="spacer"/>
   <button className="icon" type="button" aria-label="Settings" title="Settings" onClick={()=>setSettingsOpen(true)}><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg></button><WindowControls/></div>
  <nav aria-label="Main navigation">{pages.map(p=><button className={page===p?"active":""} onClick={()=>open(p)} key={p}>{p}{badges[p]&&<span className="badge">{badges[p]}</span>}</button>)}
   <span className="nav-right"><span className="count" title={barText} aria-live="polite">{barText}</span>
    {updating
     ?<button className="small danger" onClick={()=>cancelScrapeAll(updateRun!).catch(error=>setUpdateStatus(String(error)))}>Cancel update</button>
     :<button className="small" onClick={updateNow}>Update now</button>}</span></nav>
  <div className="pane">{visited.map(p=><div key={p} hidden={p!==page}>{views[p]}</div>)}
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
//
// A handful of boards fit as chips. Past that the chips were a second toolbar row of ten names and a
// "+38 more" chip, so a longer list is one control in the filter row that opens a chooser instead.
const CHIP_LIMIT=10;
function SourcePicker({sources,shown,onShow}:{sources:Source[];shown:Set<string>;onShow:(ids:string[])=>void}){
 const [pickerOpen,setPickerOpen]=useState(false);
 const scrapable=sources.filter(s=>s.kind!=="reference"&&s.enabled).sort((a,b)=>a.name.localeCompare(b.name));
 if(!scrapable.length)return null;
 const toggle=(source:Source,checked:boolean)=>onShow(checked?[...shown,source.id]:[...shown].filter(id=>id!==source.id));
 const every=()=>onShow(scrapable.map(s=>s.id)),none=()=>onShow([]);
 if(scrapable.length>CHIP_LIMIT)return <div className="source-control"><span className="lbl">Sources</span>
  <button type="button" className="select-like" onClick={()=>setPickerOpen(true)} aria-haspopup="dialog">
   {shown.size===scrapable.length?`All ${scrapable.length}`:`${shown.size} of ${scrapable.length}`}</button>
  {pickerOpen&&<SourceChooser sources={scrapable} shown={shown} onToggle={toggle} onAll={every} onNone={none} onClose={()=>setPickerOpen(false)}/>}</div>;
 return <fieldset className="chips"><legend><span className="lbl">Sources</span><span className="count">{shown.size} of {scrapable.length} shown</span>
   <button type="button" className="link" onClick={every}>All</button><button type="button" className="link" onClick={none}>None</button></legend>
  {scrapable.map(s=><label className={shown.has(s.id)?"chip on":"chip"} key={s.id}>
   <input type="checkbox" checked={shown.has(s.id)} onChange={e=>toggle(s,e.target.checked)}/>{s.name}<span className="n">{s.jobCount.toLocaleString()}</span></label>)}
 </fieldset>}
// A real modal: it is always as tall as the window allows, so Done can never end up below the fold,
// and Escape or a click beside it closes it like every other dialog in the app.
function SourceChooser({sources,shown,onToggle,onAll,onNone,onClose}:{sources:Source[];shown:Set<string>;onToggle:(source:Source,checked:boolean)=>void;onAll:()=>void;onNone:()=>void;onClose:()=>void}){
 const dialog=useRef<HTMLDialogElement>(null);const [query,setQuery]=useState("");
 useEffect(()=>{const node=dialog.current;node?.showModal?.();return()=>node?.close?.()},[]);
 const filtered=sources.filter(source=>source.name.toLowerCase().includes(query.trim().toLowerCase()));
 return <dialog ref={dialog} className="company-picker source-chooser" aria-labelledby="source-chooser-title"
   onCancel={event=>{event.preventDefault();onClose()}} onClick={event=>{if(event.target===event.currentTarget)onClose()}}>
  <div className="sheet"><header><h3 id="source-chooser-title">Sources to show</h3><button type="button" className="link" onClick={onClose}>Close</button></header>
   <div className="dialog-body"><p className="hint">This only changes what the Jobs list shows. Every source stays switched on and keeps being read.</p>
    <div className="top"><input autoFocus type="search" value={query} onChange={event=>setQuery(event.target.value)} placeholder="Filter sources by name" aria-label="Filter sources by name"/>
     <span className="count">{shown.size} of {sources.length} shown</span></div>
    <div className="list">{filtered.map(source=><label key={source.id}><input type="checkbox" checked={shown.has(source.id)} onChange={event=>onToggle(source,event.target.checked)}/><span>{source.name}</span><span className="n">{source.jobCount.toLocaleString()}</span></label>)}
     {!filtered.length&&<span className="muted">No source matches.</span>}</div></div>
   <footer><button type="button" className="small" onClick={onAll}>Show all</button><button type="button" className="small" onClick={onNone}>Show none</button><span className="spacer"/><button type="button" className="small primary" onClick={onClose}>Done</button></footer></div></dialog>}
// One line of news under the toolbar. It goes away on its own after a while — unless it is the progress of
// an update still running — and an action that can be taken back offers Undo for as long as it shows.
export type Notice={text:string;undo?:()=>Promise<unknown>;sticky?:boolean};
export type Notify=(text:string,undo?:()=>Promise<unknown>)=>void;
function NoticeLine({notice,onClear}:{notice?:Notice;onClear:()=>void}){
 const [undoing,setUndoing]=useState(false);
 useEffect(()=>{if(!notice||notice.sticky)return;const timer=setTimeout(onClear,notice.undo?15_000:10_000);return()=>clearTimeout(timer)},[notice,onClear]);
 if(!notice)return null;
 const undo=notice.undo;
 return <output className="status">{notice.text}{undo&&<>{" "}<button type="button" className="link undo" disabled={undoing}
  onClick={()=>{setUndoing(true);undo().then(onClear).catch(()=>{}).finally(()=>setUndoing(false))}}>Undo</button></>}</output>}
function useJobActions(job:Job,onStatus:Notify){
 const qc=useQueryClient();const [busy,setBusy]=useState(false);
 // Invalidate, not remove: removing the cache blanked the list back to the loading skeleton for
 // every page already scrolled through, so acting on one row threw the reader back to the top of
 // the list. Invalidating keeps the rows on screen while it refetches quietly underneath them —
 // and still lands on a correct, gap-free result: an infinite query's invalidation refetches every
 // loaded page in order, recomputing each page's offset from the one before it, so a dismissal that
 // shrinks the filtered set does not duplicate or skip a row the way stale fixed offsets would.
 const refresh=()=>{refreshJobs(qc);qc.invalidateQueries({queryKey:["apps"]})};
 const act=(open:boolean)=>{setBusy(true);const before=job.applicationStage;
  // Save and Apply are the same save; Apply also opens the posting and counts as applied — there
  // is no "Did you submit?" follow-up, so the message right after it offers Undo instead. Undo puts
  // back exactly what was there: no application at all, or the saved one this was.
  api.createApp(job.id).then(application=>(open?api.openApply(application.id):Promise.resolve()).then(()=>application))
   .then(application=>{refresh();
    const undo=isApplied(before)?undefined:()=>(before?api.stage(application.id,"planned","Undid Apply on the Jobs page",true):api.deleteApplication(application.id)).then(refresh);
    onStatus(open?`Opened ${job.title} in your browser and marked it as applied.`:`Saved ${job.title} to Applications.`,undo)})
   .catch(error=>onStatus(String(error))).finally(()=>setBusy(false))};
 // "Not for me". Reversible, and the row is only hidden — the listing stays scraped, searchable
 // and counted, which is what makes dismissing safe to do quickly.
 const dismiss=(dismissed:boolean)=>{setBusy(true);
  setJobDismissed(job.id,dismissed)
   .then(()=>{refresh();onStatus(dismissed?`Hid ${job.title}.`:`${job.title} is back in the list.`,()=>setJobDismissed(job.id,!dismissed).then(refresh))})
   .catch(error=>onStatus(String(error))).finally(()=>setBusy(false))};
 return{busy,act,dismiss};}
const jobSalary=(job:Job)=>job.salaryMin||job.salaryMax?`${job.salaryMin??"?"}–${job.salaryMax??"?"} ${job.salaryCurrency??""}`.trim():"not listed";
// How old a posting is says more down a long list than an ISO date; the date itself stays in the tooltip.
export function postedAgo(posted:string|undefined,now=Date.now()){
 if(!posted)return "not listed";
 const at=new Date(/^\d{4}-\d{2}-\d{2}$/.test(posted)?`${posted}T00:00:00`:posted);
 if(Number.isNaN(at.getTime()))return posted;
 const days=Math.round((new Date(now).setHours(0,0,0,0)-new Date(at).setHours(0,0,0,0))/86_400_000);
 if(days<=0)return "today";if(days===1)return "yesterday";if(days<14)return `${days} d ago`;
 if(days<60)return `${Math.floor(days/7)} w ago`;if(days<365)return `${Math.floor(days/30)} mo ago`;
 return at.toLocaleDateString([],{year:"numeric",month:"short"});
}
const availabilityLabel=(availability:Job["availability"])=>availability==="possibly_closed"?"May be gone":availability==="closed"?"Closed":availability==="archived"?"Archived":"Open";
export const JobCard=React.memo(function JobCard({job,sourceName,onStatus,onOpen}:{job:Job;sourceName?:string;onStatus:Notify;onOpen?:(job:Job)=>void}){
 const {busy,act,dismiss}=useJobActions(job,onStatus);const stage=job.applicationStage;
 const gone=job.availability==="closed"||job.availability==="possibly_closed"?availabilityLabel(job.availability):null;
 const open=()=>onOpen?.(job);
 return <div className="job" role="button" tabIndex={0} onClick={open} onKeyDown={event=>{if(event.target===event.currentTarget&&(event.key==="Enter"||event.key===" ")){event.preventDefault();open()}}}>
   <span className="job-title">{job.title}</span>
   <span className="job-meta">{job.company}{sourceName&&sourceName!==job.company?` · ${sourceName}`:""}</span>
   <span className="job-meta dim hide-sm">{job.location||"Location not listed"}</span>
   <span className="job-posted hide-sm" title={job.postedAt}>{postedAgo(job.postedAt)}</span>
   <span className="job-meta dim only-wide">{jobSalary(job)}</span>
   <span className="job-state hide-md">
    {gone&&<span className="pill" title={job.availability==="closed"
     ?"Two complete reads of this board did not list this job."
     :"The last complete read of this board did not list this job."}>{gone}</span>}
    {stage&&<span className={isApplied(stage)?"pill applied":"pill good"}>{stageLabel(stage)}</span>}
   </span>
   <span className="row-actions">{job.dismissedAt
     ? <button className="small" disabled={busy} onClick={event=>{event.stopPropagation();dismiss(false)}}>Bring back</button>
     : <>
      {/* Judging a listing has to be as cheap as reading it, or a list this long never gets shorter. */}
      {!stage&&<button className="small quiet" disabled={busy} title="Hide this from the list. Nothing is deleted, and you can bring it back."
       onClick={event=>{event.stopPropagation();dismiss(true)}}>Not for me</button>}
      <button className="small" disabled={busy} onClick={event=>{event.stopPropagation();if(stage)open();else act(false)}}>{stage?"Open":"Save"}</button>
      {!isApplied(stage)&&<button className="small primary" disabled={busy||!job.applyUrl} title={job.applyUrl?"Opens the posting in your browser and marks the job as applied":"This listing has no apply link"}
       onClick={event=>{event.stopPropagation();act(true)}}>Apply</button>}
     </>}</span>
  </div>});
// Reading is where most judging happens, so the reader can do everything a row can — including "Not
// for me", which drops the job and shows the next one — and moves with the keyboard.
const typing=(target:EventTarget|null)=>target instanceof Element&&!!target.closest("input,textarea,select,[contenteditable=true]");
export function JobReader({jobs,index,sourceName,onMove,onClose,onStatus}:{jobs:Job[];index:number;sourceName:(job:Job)=>string|undefined;onMove:(index:number)=>void;onClose:()=>void;onStatus:Notify}){
 const job=jobs[index];const [description,setDescription]=useState<JobDescription>();const [descriptionLoading,setDescriptionLoading]=useState(false);const descriptionRequest=useRef(0);const {busy,act,dismiss}=useJobActions(job,onStatus);
 const fetchDescription=()=>{const request=++descriptionRequest.current;setDescriptionLoading(true);jobDescription(job.id,true).then(value=>{if(request===descriptionRequest.current)setDescription(value)}).catch(error=>{if(request===descriptionRequest.current)setDescription(previous=>({text:previous?.text||"",status:"failed",error:String(error)}))}).finally(()=>{if(request===descriptionRequest.current)setDescriptionLoading(false)})};
 useEffect(()=>{const request=++descriptionRequest.current;setDescription(undefined);setDescriptionLoading(true);jobDescription(job.id).then(value=>{if(request===descriptionRequest.current)setDescription(value)}).catch(error=>{if(request===descriptionRequest.current)setDescription({text:"",status:"failed",error:String(error)})}).finally(()=>{if(request===descriptionRequest.current)setDescriptionLoading(false)});return()=>{descriptionRequest.current+=1}},[job.id]);
 // Left/right and j/k move; up and down stay free to scroll the description.
 useEffect(()=>{const keys=(event:KeyboardEvent)=>{if(typing(event.target)||event.metaKey||event.ctrlKey||event.altKey)return;
  if((event.key==="ArrowRight"||event.key==="j")&&index<jobs.length-1){event.preventDefault();onMove(index+1)}
  if((event.key==="ArrowLeft"||event.key==="k")&&index>0){event.preventDefault();onMove(index-1)}};
  window.addEventListener("keydown",keys);return()=>window.removeEventListener("keydown",keys)},[index,jobs.length,onMove]);
 const stage=job.applicationStage;
 return <section className="reader" aria-label="Job details"><header><div className="top"><h2>{job.title}</h2><span className="nav-pair"><button className="small" disabled={!index} title="Previous job (← or k)" onClick={()=>onMove(index-1)}>← Previous</button><button className="small" disabled={index>=jobs.length-1} title="Next job (→ or j)" onClick={()=>onMove(index+1)}>Next →</button><button className="small" title="Back to the list (Esc)" onClick={onClose}>Close</button></span></div>
   <div className="sub"><span>{job.company}</span><span>·</span><span>{job.location||"Location not listed"}</span><span>·</span><span title={job.postedAt}>posted {job.postedAt?postedAgo(job.postedAt):"date not listed"}</span></div></header>
  <div className="body"><div className="measure"><div className="facts"><div><span>Work mode</span><strong>{job.workMode||"not listed"}</strong></div><div><span>Seniority</span><strong>{job.seniority||"not listed"}</strong></div><div><span>Salary</span><strong>{jobSalary(job)}</strong></div><div><span>Source</span><strong>{sourceName(job)||job.company}</strong></div><div><span>Listing</span><strong>{availabilityLabel(job.availability)}</strong></div>{stage&&<div><span>Application</span><strong>{stageLabel(stage)}</strong></div>}</div>
   {description===undefined?<p className="description muted">Loading the description…</p>:description.status==="pending"?<p className="description muted">This listing has no downloadable description.</p>:description.status==="failed"?<><p className="description">{description.text}</p><p className="description warning">Description download failed{description.error?`: ${description.error}`:"."} <button className="link" disabled={descriptionLoading} onClick={fetchDescription}>Retry</button></p></>:<p className="description">{description.text||"This listing published no description."}</p>}<button className="small" disabled={descriptionLoading} onClick={fetchDescription}>{descriptionLoading?"Checking description…":"Refresh description"}</button></div></div>
  <footer>{!isApplied(stage)&&<button className="primary" disabled={busy||!job.applyUrl} title={job.applyUrl?"Opens the posting in your browser and marks the job as applied":"This listing has no apply link"} onClick={()=>act(true)}>Apply — opens the posting</button>}
   {!stage&&<button disabled={busy} onClick={()=>act(false)}>Save for later</button>}
   {!stage&&(job.dismissedAt?<button disabled={busy} onClick={()=>dismiss(false)}>Bring back</button>:<button className="quiet" disabled={busy} title="Hide this job and show the next one" onClick={()=>dismiss(true)}>Not for me</button>)}
   {job.canonicalUrl&&<a className="link" href={job.canonicalUrl} target="_blank" rel="noreferrer">Open listing in browser</a>}<span className="spacer"/><span className="count">{index+1} of {jobs.length}</span></footer></section>}
// A link someone sent you is not a board. This reads that one page, shows what it found, and lets
// the person who pasted it fix what the page did not say — which is most of it when the site
// publishes no JobPosting markup at all.
function CaptureJob({onClose,onSaved}:{onClose:()=>void;onSaved:(message:string)=>void}){
 const [url,setUrl]=useState("");const [draft,setDraft]=useState<CapturedJob>();
 const [busy,setBusy]=useState(false);const [error,setError]=useState("");
 const read=()=>{if(!/^https?:/i.test(url.trim()))return setError("Paste the full web address of the vacancy.");
  setBusy(true);setError("");
  captureJob(url.trim())
   .then(events=>{const final=events.at(-1);
    if(!final||final.event!=="completed")throw Error(describeFailure(final?.payload?.code as string|undefined));
    setDraft(final.payload as unknown as CapturedJob)})
   .catch(problem=>setError(String(problem))).finally(()=>setBusy(false))};
 const save=()=>{if(!draft)return;setBusy(true);
  saveCapturedJob(draft).then(()=>{onSaved(`Saved "${draft.title}".`);onClose()})
   .catch(problem=>setError(String(problem))).finally(()=>setBusy(false))};
 const field=(label:string,key:keyof CapturedJob,placeholder="")=>
  <label key={key}><span>{label}</span>
   <input value={String(draft?.[key]??"")} placeholder={placeholder}
    onChange={event=>setDraft(current=>current&&{...current,[key]:event.target.value})}/></label>;
 return <dialog open className="confirm" aria-label="Add a job from a link" onKeyDown={event=>{if(event.key==="Escape"){event.stopPropagation();onClose()}}}>
  <div className="sheet"><div className="dialog-body">
   <h2>Add a job from a link</h2>
   {error&&<output className="status">{error}</output>}
   <label><span>Vacancy address</span>
    <input autoFocus value={url} placeholder="https://company.com/careers/job/1234"
     onChange={event=>setUrl(event.target.value)}
     onKeyDown={event=>{if(event.key==="Enter"){event.preventDefault();read()}}}/></label>
   {!draft
    ? <p className="hint">The page is read once, here. Nothing is stored until you have seen what it found.</p>
    : <>
      {!draft.structured&&<p className="hint warning">This page publishes no job markup, so only the title could be read with any confidence. Check the fields below before saving.</p>}
      <div className="field-grid">{field("Title","title")}{field("Company","company")}{field("Location","location","not listed")}{field("Posted","postedAt","YYYY-MM-DD")}</div>
      {draft.descriptionText&&<p className="hint">{draft.descriptionText.slice(0,240)}{draft.descriptionText.length>240?"…":""}</p>}
     </>}
  </div>
  <footer><button className="link" onClick={onClose}>Cancel</button>
   {draft
    ? <button className="primary" disabled={busy||!draft.title.trim()} onClick={save}>{busy?"Saving…":"Save job"}</button>
    : <button className="primary" disabled={busy||!url.trim()} onClick={read}>{busy?"Reading…":"Read the page"}</button>}</footer></div></dialog>}
// The first thing a new install needs is something to follow. This says so once, with the choices as
// buttons, instead of pointing at a drop-down in the title bar from under an empty filter bar.
function GetStarted({onBrowse,onStatus}:{onBrowse:()=>void;onStatus:(text:string)=>void}){
 const {data}=useQuery({queryKey:["profiles"],queryFn:listProfiles});const choose=useChooseProfile(onStatus);
 const profiles=data?.profiles??[];
 return <div className="empty get-started"><strong>Choose what to follow</strong>
  <span>{profiles.length?"A profile adds a ready-made set of employers and reads them straight away. Add or remove companies any time on the Sources tab.":"Pick the companies whose career sites you want to follow."}</span>
  <div className="actions">{profiles.map(profile=><button key={profile.id} className="primary" disabled={choose.isPending} title={profile.description} onClick={()=>choose.mutate(profile.id)}>{profile.name} · {profile.companies} companies</button>)}
   <button disabled={choose.isPending} onClick={onBrowse}>Browse companies</button>
   {choose.isPending&&<span className="muted">Setting up…</span>}</div></div>}
// The filters are kept in the settings table, so the page opens on what it was last left on — after
// closing the app, and after restarting the computer. They are read before the page first renders:
// starting from the defaults and swapping the saved filter in afterwards would flash the wrong
// list and run a query for it.
//
// They are kept per profile, and the page is rebuilt from them on every profile switch: each search
// (titles, countries, order, toggles, saved views) is there exactly as it was left, and nothing typed
// for one profile leaks into the other. The read is never cached, because the copy a cache would hold
// is the one from before the last visit's edits.
type JobsProps={onCount:(count:number)=>void;onBrowse?:()=>void;onOpenSources?:()=>void;onStatus?:(text:string)=>void};
export function Jobs(props:JobsProps){
 const profiles=useQuery({queryKey:["profiles"],queryFn:listProfiles});
 const profile=profiles.data?.active??"",known=profiles.isSuccess||profiles.isError;
 const {data,isError}=useQuery({queryKey:["active-filter",profile],queryFn:()=>readProfileSetting(getSetting,ACTIVE_FILTER_KEY,profile).then(parseActiveFilter),staleTime:0,gcTime:0,retry:false,enabled:known});
 // Right after a switch the source list still holds the previous profile's boards until it reloads. A page
 // built on the old list would list nothing, offer the old profile's countries, and drop a saved country the
 // new profile's jobs do have — so a newly chosen profile's page waits for that one reload. Later refreshes
 // of the list never take the page down.
 const sourcesLoading=useIsFetching({queryKey:["sources"]})>0,built=useRef<string|undefined>(undefined);
 // If the setting cannot be read the page still opens, on the defaults.
 if(!known||(data===undefined&&!isError)||(built.current!==profile&&sourcesLoading))return null;
 built.current=profile;
 return <JobsView key={profile} profile={profile} initial={data??null} {...props}/>}
// Whether the filter area is open is how this window is used, not part of a search: kept in the browser.
const FILTERS_OPEN_KEY="jobs.filtersOpen";
const readFiltersOpen=()=>{try{return localStorage.getItem(FILTERS_OPEN_KEY)!=="false"}catch{return true}};
function JobsView({profile,onCount,initial,onBrowse,onOpenSources,onStatus}:JobsProps&{profile:string;initial:ActiveFilter|null}){
 const qc=useQueryClient();
 const [title,setTitle]=useState(initial?.title??""),[keyword,setKeyword]=useState(initial?.keyword??""),[sort,setSort]=useState<JobFilter["sort"]>(initial?.sort??"recent"),[savedOnly,setSavedOnly]=useState(initial?.savedOnly??false),[includeClosed,setIncludeClosed]=useState(initial?.includeClosed??false);
 // Countries are matched on the codes worked out when each job was stored, so this sends codes and
 // not city names. A listing whose country could not be worked out at all is left out while a
 // country filter is on — it is the one that would otherwise show up under every country.
 const [includeDismissed,setIncludeDismissed]=useState(false);const [capturing,setCapturing]=useState(false);
 const [countries,setCountries]=useState<string[]>(initial?.countries??[]),[includeUnknownLocations,setIncludeUnknownLocations]=useState(initial?.includeUnknownLocations??false);
 const [postedWithin,setPostedWithin]=useState<number|null>(initial?initial.postedWithinDays:DEFAULT_POSTED_WINDOW);
 // undefined means "not narrowed yet", which resolves to every followed source once they load.
 // Kept out of the database on purpose: this is where you are looking, not what you follow. The page is
 // rebuilt on a profile switch, so this starts again from every source of the new profile.
 const [shown,setShown]=useState<string[]>();
 const {data:sourceList=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources,refetchOnWindowFocus:true});
 const followed=sourceList.filter(s=>s.enabled&&s.kind!=="reference");
 const anySources=sourceList.some(s=>s.kind!=="reference");
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
 const [notice,setNotice]=useState<Notice>();const clearNotice=useCallback(()=>setNotice(undefined),[]);
 const setStatus=useCallback<Notify>((text,undo)=>setNotice({text,undo}),[]);
 // "New" means new to the reader, not new to the board: the moment they last said they were done
 // is kept in settings, so closing the app does not turn everything unread again.
 const {data:reviewedAt}=useQuery({queryKey:["reviewed-at",profile],queryFn:()=>readProfileSetting(getSetting,REVIEWED_KEY,profile)});
 const [onlyNew,setOnlyNew]=useState(false);
 const markReviewed=()=>setSetting(profileKey(REVIEWED_KEY,profile),new Date().toISOString())
  .then(()=>{setOnlyNew(false);qc.invalidateQueries({queryKey:["reviewed-at"]});refreshJobs(qc);
   setStatus("Marked as seen. What arrives from now on is what counts as new.")})
  .catch(error=>setStatus(String(error)));
 const filter:JobFilter={title:useDebounced(title),keyword:useDebounced(keyword),sort,savedOnly,includeClosed,postedWithinDays:postedWithin,sourceIds:shownIds,countries,includeUnknownLocations,includeDismissed,firstSeenAfter:onlyNew?reviewedAt??null:null};
 const {data,isLoading,fetchNextPage,hasNextPage,isFetchingNextPage}=useInfiniteQuery({queryKey:["jobs",filter],queryFn:({pageParam})=>api.jobs(filter,pageParam),initialPageParam:0,getNextPageParam:last=>last.hasMore?last.offset+last.items.length:undefined,refetchOnWindowFocus:true});
 const jobs=data?.pages.flatMap(page=>page.items)??[],total=data?.pages[0]?.total??0;
 // Counted with the list's own filters, so "12 new" opens on exactly twelve rows. Under the "jobs" key,
 // so everything that refreshes the list refreshes this with it.
 const newFilter={...filter,firstSeenAfter:reviewedAt??null};
 const {data:newCount=0}=useQuery({queryKey:["jobs","new-count",newFilter],queryFn:()=>countJobs(newFilter),enabled:!!reviewedAt&&!!shownIds.length});
 const [batch,setBatch]=useState<string>();const runStarted=useRef<Record<string,number>>({});
 const [toolbarOpen,setToolbarOpen]=useState(readFiltersOpen);const [readerIndex,setReaderIndex]=useState<number|null>(null);
 const toggleToolbar=()=>setToolbarOpen(open=>{try{localStorage.setItem(FILTERS_OPEN_KEY,String(!open))}catch{/* a convenience only */}return !open});
 const names=new Map(sourceList.map(s=>[s.id,s.name]));
 const selected=followed.filter(s=>shownIds.includes(s.id));
 // Every update's progress shows here — the automatic one at launch, Update now, Re-read all, and the first
 // read of a newly chosen profile — with a way to stop it, whichever button started it.
 useEffect(()=>{let active=true;const stop=listen<{runId:string;phase?:string;completed?:number;current?:number;total?:number;source?:string;requests?:number;failed?:number;cancelled?:number}>("scrape-all-progress",event=>{
  const progress=event.payload;if(!active||!progress.runId)return;const runId=progress.runId;
  runStarted.current[runId]??=Date.now();
  const elapsed=` · ${elapsedSince(runStarted.current[runId])}`;
  if(!progress.phase){refreshJobs(qc);qc.invalidateQueries({queryKey:["sources"]});const complete=(progress.completed??0)>=(progress.total??1);
   if(complete){delete runStarted.current[runId];setBatch(current=>current===runId?undefined:current);
    setNotice({text:progress.cancelled?`Update cancelled${elapsed}.`:`${progress.failed?`Update finished; ${progress.failed} source${progress.failed===1?"":"s"} could not be read${elapsed}.`:`Jobs are up to date${elapsed}.`} Descriptions download when opened.`})}
   else{setBatch(runId);setNotice({text:`Updating sources · ${progress.completed}/${progress.total}${elapsed}`,sticky:true})}}
  if(progress.phase==="source-progress"){setBatch(runId);setNotice({text:`Reading ${progress.source||"source"} · ${progress.requests||0} requests${elapsed}`,sticky:true})}
 });stop.then(()=>active&&startAutomaticSync().catch(error=>setStatus(String(error))));return()=>{active=false;stop.then(unlisten=>unlisten()).catch(()=>{})}},[qc,setStatus]);
 // "Filtered" means the view differs from the default one, which is what makes the empty state
 // honest: hiding closed jobs is the default, so turning them back on counts as a change too.
 const filtered=!!(title.trim()||keyword.trim()||savedOnly||includeClosed||countries.length||includeUnknownLocations||postedWithin!==DEFAULT_POSTED_WINDOW);
 // Whatever the filters are now is what the next launch opens on. Typing waits for a pause so a
 // word is stored once, not once per letter. The saved value is also what a launch reads, so it
 // is written only after the page has started from it — never over it with the defaults.
 const activeFilter=serializeActiveFilter({title,keyword,sort,savedOnly,includeClosed,postedWithinDays:postedWithin,countries,includeUnknownLocations});
 const filterKey=profileKey(ACTIVE_FILTER_KEY,profile),unsaved=useRef<string|undefined>(undefined);
 useEffect(()=>{unsaved.current=activeFilter;const timer=setTimeout(()=>{unsaved.current=undefined;setSetting(filterKey,activeFilter).catch(()=>{})},300);return()=>clearTimeout(timer)},[activeFilter,filterKey]);
 // Switching profile straight after typing must not lose the last word: what was still waiting is written now.
 useEffect(()=>()=>{if(unsaved.current!==undefined)setSetting(filterKey,unsaved.current).catch(()=>{})},[filterKey]);
 // Saved views: a named filter you can switch to deliberately, for the ones you use day after day.
 const {data:savedViews=[]}=useQuery({queryKey:["saved-views",profile],queryFn:()=>readProfileSetting(getSetting,PRESETS_KEY,profile).then(parsePresets)});
 const [naming,setNaming]=useState<string>();
 const storeViews=(next:FilterPreset[])=>setSetting(profileKey(PRESETS_KEY,profile),JSON.stringify(next))
  .then(()=>qc.invalidateQueries({queryKey:["saved-views"]})).catch(error=>setStatus(String(error)));
 const applyView=(view:FilterPreset)=>{setTitle(view.title);setKeyword(view.keyword);setSort(view.sort);
  setSavedOnly(view.savedOnly);setIncludeClosed(view.includeClosed);setPostedWithin(view.postedWithinDays);
  setCountries(view.countries);setIncludeUnknownLocations(view.includeUnknownLocations);
  setStatus(`Showing "${view.name}".`)};
 const saveView=(name:string)=>{if(!name.trim())return;
  storeViews(withPreset(savedViews,presetFrom(name,filter)));setNaming(undefined)};
 const clearFilters=()=>{setTitle("");setKeyword("");setSavedOnly(false);setIncludeClosed(false);setCountries([]);setIncludeUnknownLocations(false);setPostedWithin(DEFAULT_POSTED_WINDOW)};
 // The nav badge counts what the page is showing, so narrowing the view moves it too.
 useEffect(()=>onCount(selected.length?total:0),[selected.length,total,onCount]);
 useEffect(()=>{const close=(event:KeyboardEvent)=>{if(event.key==="Escape"&&!document.querySelector("dialog[open],.panel-sheet"))setReaderIndex(null)};window.addEventListener("keydown",close);return()=>window.removeEventListener("keydown",close)},[]);
 // A new install has nothing to filter: one clear way in, and none of the controls that would only be empty.
 if(!anySources)return <div className="view"><div className="toolbar"><header><h2>Jobs</h2></header></div>
  <div className="scroll"><GetStarted onBrowse={()=>onBrowse?.()} onStatus={text=>onStatus?onStatus(text):setStatus(text)}/></div></div>;
 return <div className="view">
  <div className={toolbarOpen?"toolbar":"toolbar toolbar-collapsed"}>
   <header><h2>Jobs</h2>
    <span className="count">{total?`${total.toLocaleString()} job${total===1?"":"s"}${jobs.length<total?` · showing ${jobs.length.toLocaleString()}`:""}`:""}</span>
    {batch&&<button className="small danger" onClick={()=>cancelScrapeAll(batch)}>Cancel update</button>}
    <button className="small" onClick={()=>setCapturing(true)} title="For a referral or a recruiter link: read one vacancy from its own page">Add from link</button>
    <button className="small" aria-expanded={toolbarOpen} onClick={toggleToolbar}>{toolbarOpen?"Hide filters":"Show filters"}</button></header>
   {!toolbarOpen&&<div className="filter-summary"><span>{filtered?<b>Filters on</b>:<b>All recent jobs</b>} · {selected.length===followed.length?`all ${followed.length} sources`:`${selected.length} of ${followed.length} sources`}</span><button className="link" onClick={toggleToolbar}>Edit filters</button>{filtered&&<button className="link" onClick={clearFilters}>Clear</button>}</div>}
   <section className="filters" aria-label="Filters">
    <SourcePicker sources={sourceList} shown={new Set(shownIds)} onShow={setShown}/>
    <label className="grow"><span>Title contains</span><input name="title" value={title} onChange={e=>setTitle(e.target.value)} placeholder="e.g. engineer"
     title="Searches job titles. Every word you type must appear; case and order are ignored."/></label>
    <label className="grow"><span>Keywords</span><input name="keyword" value={keyword} onChange={e=>setKeyword(e.target.value)} placeholder="e.g. python, remote"
     title="Searches title, company and location — and the description of every job you have opened. Every word must appear; case and order are ignored."/></label>
    <label><span>Posted within</span><select value={String(postedWithin)} onPointerDown={showSelect} onChange={e=>setPostedWithin(e.target.value==="null"?null:Number(e.target.value))}>
     {POSTED_WINDOWS.map(window=><option key={window.label} value={String(window.days)}>{window.label}</option>)}</select></label>
    <label><span>Countries</span>
     <select value="" disabled={!options.length} onPointerDown={showSelect} onChange={event=>addCountry(event.target.value)}>
      <option value="">{options.length?(countries.length?"Add another…":"Anywhere"):"Nothing to filter yet"}</option>
      {options.filter(entry=>!countries.includes(entry.code)).map(entry=>
       <option key={entry.code} value={entry.code}>{entry.name} ({entry.jobs})</option>)}</select></label>
    <label><span>Order</span><select value={sort} onPointerDown={showSelect} onChange={e=>setSort(e.target.value as JobFilter["sort"])}>
     <option value="recent">Newest found</option><option value="posted">Newest posted</option></select></label>
    {!!countries.length&&<div className="chosen"><span className="chosen-label">Countries</span>{countries.map(code=>
     <button key={code} type="button" onClick={()=>setCountries(current=>current.filter(entry=>entry!==code))} title={`Stop showing ${countryName(code)}`}>{countryName(code)} <span className="x">×</span></button>)}
     {!!unplaced&&<label className="inline" title="Some boards publish a count instead of a place and link to no office either, so nothing says which country they are in."><input type="checkbox" checked={includeUnknownLocations} onChange={e=>setIncludeUnknownLocations(e.target.checked)}/> Include {unplaced} with no known country</label>}</div>}
    <div className="filter-options">
     <label className="inline"><input type="checkbox" checked={savedOnly} onChange={e=>setSavedOnly(e.target.checked)}/> Saved and applied only</label>
     <label className="inline" title="Jobs their board has not listed in two complete reads are hidden by default."><input type="checkbox" checked={includeClosed} onChange={e=>setIncludeClosed(e.target.checked)}/> Include closed</label>
     <label className="inline" title="Jobs you marked Not for me are hidden. Nothing was deleted."><input type="checkbox" checked={includeDismissed} onChange={e=>setIncludeDismissed(e.target.checked)}/> Show Not for me</label>
     <span className="saved-views">
      {savedViews.map(view=><span key={view.name} className="saved-view">
       <button type="button" onClick={()=>applyView(view)} title={describePreset(view,countryName)}>{view.name}</button>
       <button type="button" className="drop" aria-label={`Delete the view "${view.name}"`}
        onClick={()=>storeViews(withoutPreset(savedViews,view.name))}>×</button></span>)}
      {naming===undefined
       ? <button type="button" className="link" disabled={!filtered}
          title={filtered?"Keep these filters under a name":"Set a filter first — a view of everything is what Clear all already gives you"}
          onClick={()=>setNaming("")}>Save view</button>
       : <input autoFocus value={naming} placeholder="Name this view" maxLength={60}
          onChange={event=>setNaming(event.target.value)}
          onBlur={()=>naming.trim()?saveView(naming):setNaming(undefined)}
          onKeyDown={event=>{if(event.key==="Enter"){event.preventDefault();saveView(naming)}
           if(event.key==="Escape"){event.stopPropagation();setNaming(undefined)}}}/>}
      {filtered&&<button type="button" className="link" onClick={clearFilters}>Clear all</button>}
     </span>
    </div>
   </section>
  </div>
  <div className="scroll">
   {capturing&&<CaptureJob onClose={()=>setCapturing(false)} onSaved={message=>{setStatus(message);{qc.removeQueries({queryKey:["jobs"]});qc.invalidateQueries({queryKey:["job-countries"]})}}}/>}
   <NoticeLine notice={notice} onClear={clearNotice}/>
   {/* The daily loop: what has arrived since you last said you were done, and a way to say it again. */}
   {!!selected.length&&(reviewedAt
    ?(!!newCount||onlyNew)&&<div className="new-since">
     <button type="button" className={onlyNew?"link on":"link"} onClick={()=>setOnlyNew(!onlyNew)}>
      {onlyNew?`Showing ${newCount.toLocaleString()} new · show all`:`${newCount.toLocaleString()} new since you last looked`}</button>
     <button type="button" className="link" onClick={markReviewed}>Mark all as seen</button></div>
    :!!total&&<div className="new-since"><button type="button" className="link" onClick={markReviewed}
      title="Marks everything listed now as seen. Jobs found after this are counted as new.">Start tracking what is new</button></div>)}
   {!followed.length?<div className="empty"><strong>Every source is switched off</strong>
     <span>Switch some back on from the Sources tab to see their jobs again. Anything you saved stays on the Applications tab.</span>
     {onOpenSources&&<button onClick={onOpenSources}>Open Sources</button>}</div>
    :!selected.length?<div className="empty"><strong>No sources shown</strong>
     <span>Nothing is listed until at least one source is shown. This only changes what you are looking at — every source stays switched on.</span>
     <button onClick={()=>setShown(followed.map(s=>s.id))}>Show all sources</button></div>
    :isLoading?<div className="skeletons" aria-hidden="true">{["58%","71%","46%","64%","52%"].map(width=>
      <div key={width}><span style={{width}}/><span/><span/><span/></div>)}</div>
    :!jobs.length?(filtered||onlyNew
     ?<div className="empty"><strong>No job matches these filters</strong>
       <span>Clear them to see everything from the selected sources.</span>
       <button onClick={()=>{clearFilters();setOnlyNew(false)}}>Clear filters</button></div>
     :<div className="empty"><strong>No jobs stored for these sources yet</strong>
       <span>JobScraper is checking these sources automatically. New listings will appear here.</span></div>)
    // The header carries the same column widths as every row, so the list reads as one table
    // and nothing shifts when the next page arrives.
    :<><div className="list-head" aria-hidden="true"><span>Role</span><span>Company</span><span className="hide-sm">Location</span><span className="hide-sm">Posted</span><span className="only-wide">Salary</span><span className="hide-md">Status</span><span>Actions</span></div>
     <div className="jobs">{jobs.map((job,index)=><JobCard job={job} sourceName={names.get(job.sourceId)} onStatus={setStatus} onOpen={()=>setReaderIndex(index)} key={job.id}/>)}</div>
     {hasNextPage&&<div className="load-more"><button disabled={isFetchingNextPage} onClick={()=>fetchNextPage()}>{isFetchingNextPage?"Loading…":`Load ${Math.min(200,total-jobs.length).toLocaleString()} more`}</button></div>}</>}
  </div>
  {readerIndex!==null&&jobs[readerIndex]&&
   <JobReader jobs={jobs} index={readerIndex} sourceName={job=>names.get(job.sourceId)}
    onMove={setReaderIndex} onClose={()=>setReaderIndex(null)} onStatus={setStatus}/>
  }
 </div>}

// ---------------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------------
function SessionBadge({sourceId}:{sourceId:string}){const {data:saved}=useQuery({queryKey:["session",sourceId],queryFn:()=>hasBrowserSession(sourceId)});return saved?<span className="tag-session">SESSION</span>:null}
const host=(url:string)=>{try{return new URL(url).hostname.replace(/^www\./,"")}catch{return url}};
// "12 min ago" says more at a glance than a timestamp, and the Jobs column already has the count.
export function readAgo(iso:string|undefined,now:number){
 if(!iso)return "";const minutes=Math.floor((now-new Date(iso).getTime())/60_000);
 if(minutes<1)return "Just now";if(minutes<60)return `${minutes} min ago`;
 if(minutes<24*60)return `${Math.floor(minutes/60)} h ago`;
 return new Date(iso).toLocaleDateString([],{month:"short",day:"numeric"});
}
// Re-renders every second while something is being read, so elapsed times and estimates move.
function useNow(fast:boolean){const [now,setNow]=useState(Date.now());useEffect(()=>{const timer=setInterval(()=>setNow(Date.now()),fast?1000:30_000);return()=>clearInterval(timer)},[fast]);return now}
// How long a finished row keeps saying what just happened before it goes back to "12 min ago".
const OUTCOME_SHOWN_MS=2*60_000;
export function SourceRow({source:s,progress,typicalMs,now,batchRunning,onTest,onEdit,onToggle,onDelete}:{source:Source;progress?:RowProgress;typicalMs?:number;now:number;batchRunning:boolean;onTest:()=>void;onEdit:()=>void;onToggle:(enabled:boolean)=>void;onDelete:()=>void}){
 const running=isRunning(progress),failed=progress?.phase==="failed";
 const recent=!!progress&&(running||batchRunning||now-(progress.finishedAt??now)<OUTCOME_SHOWN_MS);
 const fraction=progress&&recent?rowFraction(progress,now,typicalMs):undefined;
 const attention=s.disabledReason&&!s.enabled;
 return <article className={["source-row",running&&"running",attention&&"attention",recent&&progress?.phase].filter(Boolean).join(" ")}>
  <div className="source-id"><div className="source-name"><strong>{s.name}</strong>
    {supportsSessionCapture(s.adapterId)&&s.kind!=="reference"&&<SessionBadge sourceId={s.id}/>}
    {s.kind==="reference"&&<span className="pill">Reference</span>}</div>
   <span className="source-url" title={s.baseUrl}>{host(s.baseUrl)}</span></div>
  <div className="source-health">{recent&&progress
   ?<span className={`run-text ${failed?"bad":""}`} title={describeRow(progress,now)}>{running&&progress.phase!=="queued"&&<i className="mark-run"/>}<span>{describeRow(progress,now)}</span></span>
   :attention?<span className="bad" title={s.disabledReason}>{s.disabledReason}</span>
   :<span title={s.lastSuccessAt?new Date(s.lastSuccessAt).toLocaleString():undefined}>{readAgo(s.lastSuccessAt,now)||"Not read yet"}</span>}</div>
  <span className="source-jobs"><strong>{s.jobCount.toLocaleString()}</strong>
   {s.closedCount>0&&<small title="Read from this source before, and no longer listed on it">{s.closedCount.toLocaleString()} closed</small>}</span>
  <label className="toggle" title={s.kind==="reference"?"Reference sources are never read":s.enabled?"On: read on every update":"Off: not read, its jobs are kept"}>
   <input type="checkbox" role="switch" aria-label={`Read ${s.name}`} checked={s.enabled} disabled={s.kind==="reference"} onChange={e=>onToggle(e.target.checked)}/><span className="track"/></label>
  <div className="actions">
   {running&&progress?.runId?<button className="small" onClick={()=>cancelScrape(progress.runId!)}>Stop</button>
    :<button className="small" disabled={running||s.kind==="reference"} onClick={onTest}>Test</button>}
   <button className="small" disabled={running} title="Name, address, adapter settings and browser session" onClick={onEdit}>Edit</button>
   <button className="small danger" disabled={running} onClick={onDelete}>Delete</button></div>
  {fraction!==undefined&&<div className={fraction===null?"row-bar indeterminate":"row-bar"} role="progressbar" aria-label={`${s.name} progress`}
    aria-valuemin={0} aria-valuemax={100} aria-valuenow={fraction===null?undefined:Math.round(fraction*100)}
    title={progress?.phase==="reading"&&typicalMs?`Estimated from how long ${s.name} usually takes (${duration(typicalMs)})`:undefined}>
   <span style={fraction===null?undefined:{width:`${fraction*100}%`}}/></div>}
 </article>}
// The whole update in one line above the table: how many sources are done, and a way to stop it.
function BatchSummary({batch,rows,now}:{batch:BatchProgress;rows:Record<string,RowProgress>;now:number}){
 const inBatch=Object.values(rows).filter(row=>row.batchId===batch.id);
 const reading=inBatch.filter(row=>["checking","reading","saving"].includes(row.phase)).length;
 const done=!!batch.finishedAt,fraction=batch.total?batch.completed/batch.total:1;
 return <section className={done?"batch done":"batch"} aria-live="polite">
  <div className="batch-text"><strong>{done?"Update finished":"Updating sources"}</strong>
   <span>{batch.completed.toLocaleString()} of {batch.total.toLocaleString()} done{reading?` · ${reading} reading now`:""}{batch.failed?` · ${batch.failed} failed`:""} · {duration((batch.finishedAt??now)-batch.startedAt)}</span>
   {done?<button className="link" onClick={()=>progressStore.dismissBatch()}>Clear</button>:<button className="small danger" onClick={()=>cancelScrapeAll(batch.id)}>Cancel update</button>}</div>
  <div className="row-bar" role="progressbar" aria-label="Update progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction*100)}><span style={{width:`${fraction*100}%`}}/></div>
 </section>}
export function CompanyPicker({sources,onClose,onManual}:{sources:Source[];onClose:()=>void;onManual:()=>void}){
 const qc=useQueryClient(),dialog=useRef<HTMLDialogElement>(null),searchInput=useRef<HTMLInputElement>(null),[search,setSearch]=useState(""),[status,setStatus]=useState("");
 const {data:companies=[],isLoading,error,refetch}=useQuery({queryKey:["company-catalog"],queryFn:listCompanyCatalog,staleTime:Infinity});
 // A native modal keeps keyboard focus inside the picker and restores it to Browse on close.
 useEffect(()=>{const node=dialog.current;node?.showModal();searchInput.current?.focus();return()=>node?.close()},[]);
 const add=useMutation({mutationFn:async(company:CatalogCompany)=>{
  const existing=catalogSource(company,sources);
  if(existing){await api.setSourceEnabled(existing.id,true);return{...existing,enabled:true,disabledReason:undefined}}
  // Omit the catalog ID: a deleted starter retains its stable ID as a tombstone. Reusing it
  // would update an invisible row instead of adding the company the user explicitly chose.
  return api.saveSource({name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,kind:company.kind,enabled:true,
   disabledReason:null,robotsOverride:true,allowPrivateNetwork:false,configJson:{schemaVersion:"1.1.0",adapterVersion:"1.1.0",mode:"direct",...company.config}})},
  onMutate:()=>setStatus(""),onSuccess:async source=>{qc.setQueryData<Source[]>(["sources"],current=>[...(current||[]).filter(item=>item.id!==source.id),source]);setStatus(`${source.name} is selected for automatic updates.`);await qc.invalidateQueries({queryKey:["sources"]})}});
 const countryName=(code:string)=>COUNTRIES.find(country=>country.code===code)?.name||code;
 const words=search.toLowerCase().trim().split(/\s+/).filter(Boolean),visible=companies.filter(company=>words.every(word=>`${company.name} ${company.country} ${countryName(company.country)} ${company.tags.join(" ")} ${(company.profiles||[]).join(" ").replaceAll("-"," ")}`.toLowerCase().includes(word))).sort((a,b)=>a.name.localeCompare(b.name));
 return <dialog ref={dialog} className="company-picker" aria-labelledby="company-picker-title" onCancel={onClose}><div className="sheet">
  <header><h3 id="company-picker-title">Browse companies</h3><button type="button" className="link" onClick={onClose}>Close</button></header>
  <div className="dialog-body"><p className="hint">Choose a company to follow. Its jobs board and settings are ready to use. Companies are listed by home country; openings may be worldwide.</p>
   <label><span>Search companies, countries or sectors</span><input ref={searchInput} type="search" value={search} onChange={event=>setSearch(event.target.value)} placeholder="Company, Canada, semiconductor…"/></label>
   {isLoading?<p className="muted">Loading companies…</p>:error?<p role="alert" className="error">Could not load companies. <button onClick={()=>refetch()}>Try again</button></p>:<>
    <span className="hint" aria-live="polite">{visible.length} of {companies.length} companies</span>
    <div className="company-list">{visible.map(company=>{const existing=catalogSource(company,sources),pending=add.isPending&&add.variables?.id===company.id;return <article className="company-row" key={company.id}>
     <div><strong>{company.name}</strong><small>{countryName(company.country)} · {company.tags.map(tag=>tag.replaceAll("-"," ")).join(" · ")}</small></div>
     <button className={existing?.enabled?"link":""} disabled={add.isPending||existing?.enabled} aria-label={`${existing?.enabled?"Added":existing?"Enable":"Add"} ${company.name}`} onClick={()=>add.mutate(company)}>{pending?"Adding…":existing?.enabled?"Added":existing?"Enable":"Add"}</button>
    </article>})}</div>{!visible.length&&<p className="muted">No matching companies. Try another search or add a source manually.</p>}</>}
  </div><footer><button className="link" onClick={onManual}>Add source manually</button><output role="status">{add.error?String(add.error):status}</output></footer>
 </div></dialog>
}
// The profile switch: which job search the whole window is about. The active profile decides the sources the
// Sources tab lists and an update reads, the jobs the Jobs tab shows and the applications the Applications tab
// holds. Switching switches nothing off and deletes nothing: the other profile's sources, jobs and applications
// are only out of view, and come back exactly as they were when it is chosen again. The first time a profile is
// chosen its companies are added and read. "All profiles" shows every profile at once.
// Choosing a profile, wherever it is chosen from: the switch in the title bar, or the first-run buttons.
function useChooseProfile(onStatus:(text:string)=>void){
 const qc=useQueryClient();
 const {data}=useQuery({queryKey:["profiles"],queryFn:listProfiles});
 const profiles=data?.profiles??[];
 // Every tab reads through the active profile, so every tab's data is stale the moment it changes.
 const refresh=async()=>{await Promise.all(["sources","profiles","apps","new-since"].map(key=>qc.invalidateQueries({queryKey:[key]})));refreshJobs(qc)};
 return useMutation({mutationFn:(id:string)=>applyProfile(id||null),
  onSuccess:async(applied,id)=>{
   const name=profiles.find(profile=>profile.id===id)?.name;
   onStatus(describeProfileApplied(name,applied));
   await refresh();
   if(!applied.readNow.length)return;
   // A profile's companies that were never read have no jobs to show, so they are read now rather than at the
   // next automatic update.
   try{const result=await scrapeAll(crypto.randomUUID(),applied.readNow,false);onStatus(`${name||"Profile"}: ${describeUpdateSummary(result)}`)}
   catch(failure){onStatus(String(failure))}
   await refresh()},
  onError:failure=>onStatus(String(failure))})}
export function ProfileSwitcher({onStatus}:{onStatus:(text:string)=>void}){
 const {data,error}=useQuery({queryKey:["profiles"],queryFn:listProfiles});
 const profiles=data?.profiles??[],active=data?.active??"",current=profiles.find(profile=>profile.id===active);
 const choose=useChooseProfile(onStatus);
 if(error)return <span className="profile-switch error" role="alert" title={String(error)}>Profiles unavailable</span>;
 if(!profiles.length)return null;
 return <label className="profile-switch" title={current?`${current.description} ${current.on} of ${current.companies} sources are switched on.`:"Every profile's sources, jobs and applications together."}>
  <span>Profile</span>
  <select value={active} disabled={choose.isPending} onPointerDown={showSelect} onChange={event=>{if(event.target.value!==active)choose.mutate(event.target.value)}}>
   {profiles.map(profile=><option key={profile.id} value={profile.id}>{profile.name}</option>)}
   <option value="">All profiles</option></select>
  {choose.isPending&&<span className="dim">Switching…</span>}</label>}
// Says on the Sources tab which profile it is showing, and where what is added here ends up.
function ProfileNote(){
 const {data}=useQuery({queryKey:["profiles"],queryFn:listProfiles});
 if(!data?.profiles.length)return null;
 const current=data.profiles.find(profile=>profile.id===data.active);
 return <p className="hint profile-note">{current
  ?<><b>{current.name}</b> profile · {current.on} of {current.companies} sources switched on. Companies you add here belong to {current.name}. Switch profile at the top of the window.</>
  :<>Showing <b>all profiles</b>. Choose a profile at the top of the window to work on one job search at a time; a company added while all profiles are shown belongs to none of them.</>}</p>}
function Sources({browseRequest=0,onStatus}:{browseRequest?:number;onStatus?:(text:string)=>void}){
 const qc=useQueryClient(); const {data:sources=[],isLoading,error}=useQuery({queryKey:["sources"],queryFn:api.listSources});
 const [selected,setSelected]=useState<Source|undefined>(); const [result,setResult]=useState(""); const [notices,setNotices]=useState<string[]>([]); const [paused,setPaused]=useState<WorkerEvent>(); const [preview,setPreview]=useState<ReturnType<typeof previewFromEvents>&{source:string}>();
 const [browsing,setBrowsing]=useState(false);
 useEffect(()=>{if(browseRequest)setBrowsing(true)},[browseRequest]);
 // robots.txt is respected unless the user says otherwise: a site that asks not to be read gets the
 // one-time question in configureSource, and the answer then applies to every source added after it.
 const manualSource=()=>{setBrowsing(false);setSelected({id:"",name:"",baseUrl:"https://",adapterId:"static-css",adapterVersion:"1.1.0",enabled:false,kind:"active",robotsOverride:false,jobCount:0,closedCount:0})};
 const [rebuilding,setRebuilding]=useState(false);const [logShown,setLogShown]=useState(false);
 const progress=useSourceProgress(),batchRunning=!!progress.batch&&!progress.batch.finishedAt;
 const anyRunning=batchRunning||Object.values(progress.rows).some(isRunning),now=useNow(anyRunning);
 // How long each source usually takes, which is the only total a board read has. Refreshed when an
 // update ends, so the next one estimates from this one too.
 const {data:performance}=useQuery({queryKey:["scrape-performance"],queryFn:scrapePerformance,staleTime:5*60_000});
 const typical=new Map((performance?.aggregates??[]).filter(row=>row.sourceId).map(row=>[`${row.action}:${row.sourceId}`,row.medianMs]));
 useEffect(()=>{if(!anyRunning)qc.invalidateQueries({queryKey:["scrape-performance"]})},[anyRunning,qc]);
 // Worker warnings are the only way the user learns that a site changed the terms — a redirect to
 // its own default location filter, a robots override, a host that moved. They are logged either
 // way, but the one that matters while adding a source has to be visible here and then.
 useEffect(()=>listenForScrapeEvents(listen,item=>{
  if(item.event==="started"&&item.command!=="check_source"&&!item.batchId)setNotices([]);
  if(item.event==="warning"&&typeof item.payload.message==="string"&&item.payload.code!=="rate_limited_backoff"){const text=item.payload.message;setNotices(current=>current.includes(text)?current:[...current,text])}
  if(item.event==="needs_user_action")setPaused(item);
  if(["completed","cancelled","failed"].includes(item.event))setPaused(current=>current?.runId===item.runId?undefined:current);
  if(item.event==="completed"&&item.command!=="check_source"){qc.invalidateQueries({queryKey:["sources"]});refreshJobs(qc)}}),[qc]);
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
   setResult(reason?`${label}: saved but left off — ${reason}`:`${label}: set up, ${found} listing${found===1?"":"s"} found.`)}
  catch(e){setResult(e instanceof Error?e.message:String(e))}};
 const test=async(source:Source)=>{setResult("");try{const events=await api.test(source);setPreview({...previewFromEvents(events),source:source.name});setResult(describeOutcome(events,source.name))}catch(e){setResult(`${source.name}: ${describeFailure()} (see Diagnostics for details)`);console.error(e)}};
 const rebuild=()=>{const runId=crypto.randomUUID(),started=Date.now();setRebuilding(true);setResult("");scrapeAll(runId,[],true)
  .then(summary=>{setResult(`${describeUpdateSummary(summary)} ${elapsedSince(started)}. Descriptions still download only when opened.`);refreshJobs(qc);qc.invalidateQueries({queryKey:["sources"]})})
  .catch(error=>setResult(String(error))).finally(()=>setRebuilding(false))};
 const toggle=(source:Source,enabled:boolean)=>{
  // Flip the switch now; waiting on the round trip made it feel like the click had missed.
  qc.setQueryData<Source[]>(["sources"],current=>current?.map(item=>item.id===source.id?{...item,enabled}:item));
  api.setSourceEnabled(source.id,enabled).then(()=>{qc.invalidateQueries({queryKey:["sources"]});refreshJobs(qc)}).catch(e=>{setResult(String(e));qc.invalidateQueries({queryKey:["sources"]})})};
 // window.confirm is not reliably honoured inside the webview; the dialog plugin always is.
 const remove=(source:Source)=>ask(`Remove ${source.name}? Its stored jobs leave the Jobs list; jobs already saved to Applications are kept.`,{title:"Remove source?",kind:"warning"})
  .then(confirmed=>confirmed?api.deleteSource(source.id).then(()=>{qc.invalidateQueries({queryKey:["sources"]});refreshJobs(qc);setResult(`Removed ${source.name}.`)}):undefined).catch(e=>setResult(String(e)));
 const hasSources=sources.some(source=>source.kind!=="reference");
 const time=(at:number)=>new Date(at).toTimeString().slice(0,8);
 return <div className={logShown?"sources-layout":"sources-layout log-hidden"}>
  <section className="sources-main">
   <header><div><h2>Sources</h2><p>Every source that is switched on is read automatically when the app opens.</p></div>
    <div className="actions"><button onClick={()=>setLogShown(shown=>!shown)}>{logShown?"Hide log":"Show log"}</button><button disabled={rebuilding||batchRunning||!hasSources} title="Read every switched-on source again in full, not only what changed" onClick={rebuild}>{rebuilding?"Re-reading…":"Re-read all"}</button>
     <button onClick={manualSource}>Add source manually</button><button className="primary" onClick={()=>setBrowsing(true)}>Browse companies</button></div></header>
   <div className="scroll">
    <ProfileNote/>
    {progress.batch&&<BatchSummary batch={progress.batch} rows={progress.rows} now={now}/>}
    {paused&&<aside className="paused" aria-live="polite"><span className="tag">PAUSED</span>
     <span className="text">{paused.sourceName?`${paused.sourceName}: login`:"Login"} or CAPTCHA needs your action in the browser window.</span>
     <button onClick={()=>cancelScrape(paused.runId)}>Cancel</button>
     <button className="primary" onClick={()=>api.resumeScrape(paused.runId)}>Resume &amp; save session</button></aside>}
    {result&&<output className="status">{result}</output>}
    {notices.map(text=><output className="status warning" key={text}>{text}</output>)}
    {isLoading?<p className="muted">Loading…</p>:!sources.length?<GetStarted onBrowse={()=>setBrowsing(true)} onStatus={text=>onStatus?onStatus(text):setResult(text)}/>:<div className="source-table">
     <div className="source-head"><span>Source</span><span>Last read</span><span className="num">Jobs</span><span>On</span><span/></div>
     {sources.map(s=>{const row=progress.rows[s.id];return <SourceRow key={s.id} source={s} progress={row} now={now} batchRunning={batchRunning&&row?.batchId===progress.batch?.id}
      typicalMs={typical.get(`${row?.phase==="testing"?"test_source":"scrape_source"}:${s.id}`)}
      onTest={()=>test(s)} onEdit={()=>setSelected(s)} onToggle={enabled=>toggle(s,enabled)} onDelete={()=>remove(s)}/>})}</div>}
    {error&&<p className="error">{String(error)}</p>}
    {preview&&<><div className="rule"><h3>Test preview · {preview.source} · not saved</h3><hr/>
      <span className="count">{preview.jobs.length} row{preview.jobs.length===1?"":"s"} · {preview.complete?"complete":"partial"}</span>
      <button onClick={()=>setPreview(undefined)}>Close preview</button></div>
     <section className="panel" aria-label="Test preview">
      {preview.warnings.length>0&&<div className="panel-note">{preview.warnings.join(" · ")}</div>}
      <table><thead><tr><th>Title</th><th>Company</th><th>Location</th><th>Date</th><th>URL</th></tr></thead>
       <tbody>{preview.jobs.map((job,index)=><tr key={index}><td>{String(job.title??"—")}</td><td>{String(job.company??"—")}</td><td>{String(job.location??"—")}</td><td>{String(job.postedAt??"—")}</td><td>{job.applyUrl||job.canonicalUrl?<a href={String(job.applyUrl??job.canonicalUrl)} target="_blank" rel="noreferrer">Open</a>:"—"}</td></tr>)}</tbody></table>
      </section></>}
    {hasSources&&<p className="hint" style={{marginTop:22}}>What each source stores is set in Settings → Collecting, separately for each profile. The filters on the Jobs page only change what you see.</p>}
   </div>
  </section>
  {logShown&&<aside className="runlog">
   <header><strong>Run log</strong><span className="caption">this session</span><button className="link close" onClick={()=>setLogShown(false)}>Hide</button></header>
   <div className="lines">{progress.log.length?progress.log.map(line=><div key={line.id}><time>{time(line.at)}</time><b title={line.source}>{line.source}</b><span className={line.tone}>{line.text}</span></div>)
    :<span className="dim">Nothing yet. Tests and updates appear here, one line per source event.</span>}</div>
   <footer><button disabled={!progress.log.length} onClick={()=>navigator.clipboard?.writeText(progress.log.map(line=>`${time(line.at)} ${line.source}: ${line.text}`).join("\n"))}>Copy log</button>
    <button disabled={!progress.log.length} onClick={()=>progressStore.clearLog()}>Clear</button></footer>
  </aside>}
  {selected&&<SourceForm source={selected} onCancel={()=>setSelected(undefined)} onSave={submit}/>}
  {browsing&&<CompanyPicker sources={sources} onClose={()=>setBrowsing(false)} onManual={manualSource}/>}
 </div>}
// Asked once, ever: after the first yes every blocked source overrides silently.
const ROBOTS_ACK="robots.autoAcknowledged";
// Layer one of the two filter layers. This one decides what is ever stored; the Jobs page
// boxes decide what is shown of what was stored. Mirrors db.rs::SCRAPE_TITLE_FILTER.
const SCRAPE_TITLE_FILTER="scrape.titleAny";
// Layer three, and the only one nobody has to be looking at the app to notice. Mirrors
// db.rs::NOTIFY_TITLE_FILTER.
const NOTIFY_TITLE_FILTER="notify.titleAny";
const configFields:Record<string,string[]>={"static-css":["itemSelector","titleSelector","companySelector","locationSelector","dateSelector","urlSelector","descriptionSelector","nextSelector","detailUrlField"],"static-xpath":["itemXPath","titleXPath","companyXPath","locationXPath","urlXPath","descriptionXPath","nextXPath"],playwright:["urlTemplate","itemSelector","titleSelector","nextSelector"],json:["urlTemplate","itemsPath","pageParam","detailUrlField"],rss:["urlTemplate"],apple:["locale","query"],workday:["tenant","site","listingPath","query","pageSize","detailUrlField","splitFacet","splitThreshold"],eightfold:["domain","eightfoldApi","query","pageSize"],icims:["listingPath","query","pageSize","detailUrlField"],"talentbrew-jibe":["listingPath","query","pageSize","detailUrlField"],phenom:["listingPath","query","pageSize","detailUrlField"],
 // Company boards derive their own URLs, so the only thing there is to set is how far to read.
 arm:["maxPages"],amd:["maxPages"],asml:[],mediatek:["maxPages"],google:["maxPages"],cisco:["maxPages"],"sk-hynix":[],"u-blox":[],
 greenhouse:["token"],ashby:["token"],lever:["token","pageSize"],workable:["token"],bamboohr:["token"],personio:["token","domain"],oracle:["apiHost","siteNumber","query","pageSize"]};
// Mirrors requiredFor() in sidecar/worker.mjs: fields an adapter cannot build a request without.
const requiredFields=(id:string)=>id==="static-css"?["itemSelector","titleSelector"]:id==="static-xpath"?["itemXPath","titleXPath"]:id==="json"?["itemsPath"]:id==="workday"?["tenant","site"]:id==="eightfold"?["domain"]:["greenhouse","ashby","lever","workable","bamboohr","personio"].includes(id)?["token"]:id==="oracle"?["apiHost","siteNumber"]:[];
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
// Saved once and applied on every automatic update. Empty always means "everything": a filter that
// silently discarded a whole run would be indistinguishable from a broken scraper. Both title
// filters — what gets stored, and what gets announced — are the same box over a different setting.
//
// Both are kept per profile, like the Jobs page's filters: a term that suits one job search would
// otherwise silently empty the other. A profile that never saved its own uses the shared value, which
// is the one edited while All profiles is chosen. db.rs reads them the same way.
function TitleFilter({settingKey,profile,placeholder,help,describe}:
 {settingKey:string;profile:string;placeholder:string;help:string;describe:(terms:string)=>string}){
 const [title,setTitle]=useState("");const [saved,setSaved]=useState("");const [status,setStatus]=useState("");
 useEffect(()=>{setStatus("");readProfileSetting(getSetting,settingKey,profile).then(stored=>{setTitle(stored??"");setSaved(stored??"")}).catch(()=>{})},[settingKey,profile]);
 const save=()=>setSetting(profileKey(settingKey,profile),title).then(()=>{setSaved(title);setStatus(describe(title.trim()))})
  .catch(e=>setStatus(e instanceof Error?e.message:String(e)));
 return <div className="setting scrape-filter">
  <label><span>Title contains</span>
   <input value={title} onChange={e=>setTitle(e.target.value)} placeholder={placeholder} title={help}
    onKeyDown={event=>{if(event.key==="Enter"&&title!==saved){event.preventDefault();save()}}}/></label>
  <small>{help}</small>
  <div className="row"><button className="primary small" disabled={title===saved} onClick={save}>Save filter</button></div>
  {status&&<output className="status">{status}</output>}
 </div>}
function Settings({open,onClose}:{open:boolean;onClose:()=>void}){
 const qc=useQueryClient();const {data}=useQuery({queryKey:["diagnostics"],queryFn:api.diagnostics,enabled:open});
 const {data:profiles}=useQuery({queryKey:["profiles"],queryFn:listProfiles,enabled:open});
 const profile=profiles?.active??"",profileName=profiles?.profiles.find(entry=>entry.id===profile)?.name;
 const scope=profileName?` · ${profileName} profile`:profiles?.profiles.length?" · shared by all profiles":"";
 const scopeHelp=profileName?` Kept for the ${profileName} profile only.`:profiles?.profiles.length?" Used by every profile that has not set its own.":"";
 const {data:background,error:backgroundError,refetch:refetchBackground}=useQuery({queryKey:["background-sync"],queryFn:backgroundSyncStatus,enabled:open});
 const {data:launchAtLogin,error:launchError,refetch:refetchLaunchAtLogin}=useQuery({queryKey:["launch-at-login"],queryFn:launchAtLoginStatus,enabled:open});
 const [message,setMessage]=useState("");const [backgroundBusy,setBackgroundBusy]=useState(false);const [launchBusy,setLaunchBusy]=useState(false);
 const [purging,setPurging]=useState(false);const [confirmation,setConfirmation]=useState("");const [purgeBusy,setPurgeBusy]=useState(false);
 const toggleBackground=(enabled:boolean)=>{setBackgroundBusy(true);setBackgroundSync(enabled).then(result=>{setMessage(result.issue??(result.enabled?"Background checks enabled.":"Background checks disabled."));return refetchBackground()}).catch(error=>setMessage(String(error))).finally(()=>setBackgroundBusy(false))};
 const toggleLaunch=(enabled:boolean)=>{setLaunchBusy(true);setLaunchAtLogin(enabled).then(result=>{setMessage(result.issue??(result.enabled?"Start at sign-in enabled.":"Start at sign-in disabled."));return refetchLaunchAtLogin()}).catch(error=>setMessage(String(error))).finally(()=>setLaunchBusy(false))};
 const closePurge=()=>{setPurging(false);setConfirmation("")};
 const purge=()=>{setPurgeBusy(true);purgeAllJobs(confirmation).then(result=>{setMessage(`Deleted ${result.deleted.toLocaleString()} stored job${result.deleted===1?"":"s"}${result.kept?`; kept ${result.kept.toLocaleString()} with an application or review`:""}.`);closePurge();for(const key of ["diagnostics","jobs","sources","apps"])qc.invalidateQueries({queryKey:[key]})}).catch(error=>setMessage(String(error))).finally(()=>setPurgeBusy(false))};
 const reconcile=()=>api.reconcileReminders().then(result=>{const counts=(result??{}) as {scheduled?:number;missed?:number;errors?:number};
  setMessage(`Reminders checked: ${counts.scheduled??0} scheduled, ${counts.missed??0} missed${counts.errors?`, ${counts.errors} could not be scheduled — see Diagnostics`:""}.`)}).catch(error=>setMessage(String(error)));
 if(!open)return null;
 return <><div className="scrim" onClick={onClose}/><aside className="panel-sheet" role="dialog" aria-modal="true" aria-label="Settings"><header><h2>Settings</h2><button className="link close" onClick={onClose}>Close</button></header>
  <div className="panel-body">{message&&<output className="status">{message}</output>}
   <section><span className="caption">Startup</span>
    <div className="setting"><label className="inline"><input type="checkbox" checked={launchAtLogin?.enabled??false} disabled={!launchAtLogin||launchBusy} onChange={event=>toggleLaunch(event.target.checked)}/> Open JobScraper when I sign in</label><small>Starts the app itself in the notification area, without a window, so it is ready when you click it.{launchAtLogin?.debugBuild?" This development build registers its debug executable.":""}</small>{launchAtLogin?.issue&&<small className="warning" role="alert">{launchAtLogin.issue}</small>}{launchError&&<small className="warning">{String(launchError)}</small>}</div>
    <div className="setting"><label className="inline"><input type="checkbox" checked={background?.enabled??false} disabled={!background||backgroundBusy} onChange={event=>toggleBackground(event.target.checked)}/> Check for new jobs in the background</label><small>Works with the app closed: a scheduled system task reads your sources at sign-in and every 4 hours, then shows one notification when new listings appear.{background?.debugBuild?" This development build uses the JobScraper-dev database.":""}</small>{background?.issue&&<small className="warning" role="alert">{background.issue}</small>}{backgroundError&&<small className="warning">{String(backgroundError)}</small>}</div>
   </section>
   <section><span className="caption">Collecting{scope}</span>
    <TitleFilter settingKey={SCRAPE_TITLE_FILTER} profile={profile} placeholder="e.g. engineer, data scientist"
     help={`Comma-separated; a term can be a phrase. Empty stores everything. This decides what is saved at all — Jobs filters only change what you see.${scopeHelp}`}
     describe={terms=>terms?`Saved. Updates will store only titles containing ${terms}.`:"Saved. Updates will store every listing."}/></section>
   <section><span className="caption">Notifications{scope}</span>
    <TitleFilter settingKey={NOTIFY_TITLE_FILTER} profile={profile} placeholder="e.g. senior, machine learning"
     help={`Comma-separated; a term can be a phrase. Empty announces every new listing. This decides what the notification after an update mentions — everything collected is still on the Jobs page.${scopeHelp}`}
     describe={terms=>terms?`Saved. Updates will announce only new titles containing ${terms}.`:"Saved. Updates will announce every new listing."}/></section>
   <section><span className="caption">Reminders</span><div className="setting"><small>Rebuild pending application reminders after changing system tasks or restoring data.</small><div className="row"><button onClick={reconcile}>Reconcile reminders</button></div></div></section>
   <section><span className="caption">Data</span><div className="setting"><small>{data?`${data.dbPath} · ${data.jobCount.toLocaleString()} listings · schema ${data.schemaVersion}`:"Loading local data…"}</small></div>
    <div className="danger-zone"><strong>Delete all stored jobs</strong><small>Sources and their settings stay, and anything saved or applied to is kept. The next update re-reads every source in full.</small><button className="small danger" onClick={()=>setPurging(true)}>Delete all jobs…</button></div></section>
  </div></aside>
  {purging&&<dialog open className="confirm" aria-label="Delete every stored job" onKeyDown={event=>{if(event.key==="Escape"){event.stopPropagation();closePurge()}}}><div className="sheet"><div className="dialog-body"><h2>Delete every stored job?</h2><p>This removes all {data?.jobCount?.toLocaleString()??""} stored listings and their sighting history. Sources, settings, and saved applications stay.</p><p className="detail">This cannot be undone.</p><label><span>Type Confirm to continue</span><input autoFocus value={confirmation} onChange={event=>setConfirmation(event.target.value)} placeholder="Confirm"/></label></div><footer><button className="link" onClick={closePurge}>Cancel</button><button className="danger" disabled={purgeBusy||confirmation.trim()!=="Confirm"} onClick={purge}>{purgeBusy?"Deleting…":"Delete all jobs"}</button></footer></div></dialog>}
 </>}
export function SourceForm({source,onCancel,onSave}:{source:Source;onCancel:()=>void;onSave:(draft:SourceDraft)=>void}){
 const qc=useQueryClient();const canCapture=Boolean(source.id&&source.kind!=="reference"&&supportsSessionCapture(source.adapterId));
 const {data:savedSession}=useQuery({queryKey:["session",source.id],queryFn:()=>hasBrowserSession(source.id),enabled:canCapture});
 const [url,setUrl]=useState(source.baseUrl==="https://"?"":source.baseUrl),[name,setName]=useState(source.name),[adapter,setAdapter]=useState(source.id?source.adapterId:""),[config,setConfig]=useState<Record<string,unknown>>({pageSize:20,maxPages:50});
 const [enabled,setEnabled]=useState(source.id?source.enabled:true),[robots,setRobots]=useState(source.robotsOverride),[local,setLocal]=useState(false),[headless,setHeadless]=useState(true),[adjust,setAdjust]=useState(false),[error,setError]=useState(""),[sessionStatus,setSessionStatus]=useState("");
 const [configReady,setConfigReady]=useState(!source.id);
 useEffect(()=>{if(source.id)sourceConfig(source.id).then(config=>{setConfig(config);setHeadless(config.headless!==false);setConfigReady(true)}).catch(()=>setError("Could not load the saved settings. Close this form and try again."))},[source.id]);
 const missing=(requiredFields(adapter)).filter(key=>!String(config[key]||"").trim());
 // An adapter chosen by hand under Adjust is taken as-is; everything else configures itself.
 const save=(event:React.FormEvent)=>{event.preventDefault();
  if(!(url.startsWith("http://")||url.startsWith("https://")))return setError("Enter the full web address of the jobs page.");
  // An existing source already has proven settings. Saving a new name must not probe again and
  // replace a catalog token with generic selectors just because Advanced stayed closed.
  const manual=!!source.id||(adjust&&!!adapter);
  if(manual&&missing.length)return setError(`${adapter} needs: ${missing.join(", ")}.`);
  setError("");
  onSave({id:source.id||undefined,kind:source.kind,name,baseUrl:url,adapterId:adapter,config,enabled,robots,local,headless,manual})};
 const capture=async()=>{setSessionStatus("Opening a browser session…");try{const events=await captureSession(source);setSessionStatus(events.at(-1)?.event==="completed"?"Encrypted session saved. Cookie values are never displayed.":describeOutcome(events,source.name));qc.invalidateQueries({queryKey:["session",source.id]})}catch(error){setSessionStatus(String(error))}};
 const forget=()=>ask("Delete the encrypted saved session for this source?",{title:"Forget session?",kind:"warning"})
  .then(confirmed=>confirmed?deleteBrowserSession(source.id).then(()=>{setSessionStatus("Saved session deleted.");qc.invalidateQueries({queryKey:["session",source.id]})}):undefined)
  .catch(error=>setSessionStatus(String(error)));
 return <dialog open onKeyDown={event=>{if(event.key==="Escape"){event.stopPropagation();onCancel()}}}><form onSubmit={save}>
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
    <label className="inline"><input type="checkbox" checked={headless} onChange={e=>setHeadless(e.target.checked)}/> Headless browser</label>
    <label className="inline" title="Read this site even where its robots.txt asks automated tools not to"><input type="checkbox" checked={robots} onChange={e=>setRobots(e.target.checked)}/> Ignore robots.txt</label>
    <label className="inline"><input type="checkbox" checked={local} onChange={e=>setLocal(e.target.checked)}/> Allow private/local network</label>
   <label className="inline"><input type="checkbox" checked={enabled} onChange={e=>setEnabled(e.target.checked)}/> Switched on — read on every update</label></section>}
   {canCapture&&<section className="advanced"><div className="actions"><span className="hint">Browser session: {savedSession?"saved":"none"}</span><button type="button" onClick={capture}>Capture session</button>{savedSession&&<button type="button" className="danger" onClick={forget}>Forget session</button>}</div>{sessionStatus&&<output className="status">{sessionStatus}</output>}</section>}
   {source.kind==="reference"&&<p className="warning">Reference source never scrapes.</p>}{error&&<p className="error">{error}</p>}
  </div>
  <footer><button type="button" onClick={onCancel}>Cancel</button>
   <button className="primary" disabled={!url||!configReady}>Save</button></footer></form></dialog>
}

// ---------------------------------------------------------------------------------------------
// Applications
// ---------------------------------------------------------------------------------------------
// Three columns, three buttons a card. Marking applied by hand counts as the user's own word, so it
// goes through as a manual override with a fixed reason; an attempt still open from the old
// "Did you submit?" flow is answered Yes instead, so it is settled rather than left behind. Rejected is
// where an application goes once it is over, which keeps Applied to the ones still in play.
const markApplied=(app:Application)=>app.pendingConfirmation
 ?api.applyDecision(app.id,"yes")
 :api.stage(app.id,"applied",isEnded(app.currentStage)?"Reopened on the Applications board":"Marked applied on the Applications board",true);
const moveBackToSaved=(app:Application)=>api.stage(app.id,"planned","Moved back to Saved",true);
const markRejected=(app:Application)=>api.stage(app.id,"rejected","Marked rejected on the Applications board",true);
const moveTo=(app:Application,column:BoardColumn)=>column==="planned"?moveBackToSaved(app):column==="applied"?markApplied(app):markRejected(app);
const movedText=(name:string,column:BoardColumn)=>column==="planned"?`Moved ${name} back to Saved.`:column==="applied"?`Marked ${name} as applied.`:`Marked ${name} as rejected.`;
export function AppCard({app,onStatus,refresh}:{app:Application;onStatus:(message:string)=>void;refresh:()=>void}){
 const drag=useDraggable({id:app.id,data:{application:app}});const [busy,setBusy]=useState(false);
 const column=boardColumn(app.currentStage),url=websiteUrl(app),name=app.title||"this job";
 const run=(work:()=>Promise<unknown>,done:string)=>{setBusy(true);
  work().then(()=>{onStatus(done);refresh()}).catch(error=>onStatus(String(error))).finally(()=>setBusy(false))};
 // Unsaving is cheap to undo — Save it again from Jobs. Removing one you applied to loses the record
 // that you applied, and when, so that one asks first.
 const remove=()=>{if(column==="planned")return run(()=>api.deleteApplication(app.id),`Unsaved ${name}.`);
  ask(`Remove ${name} from Applications? The record that you applied goes with it.`,{title:"Remove application?",kind:"warning"})
   .then(confirmed=>{if(confirmed)run(()=>api.deleteApplication(app.id),`Removed ${name}.`)})};
 const appliedOn=app.appliedAt?`applied ${new Date(app.appliedAt).toLocaleDateString()}`:"applied";
 const when=column==="planned"?"saved":column==="applied"?appliedOn:`${stageLabel(app.currentStage).toLowerCase()}${app.appliedAt?` · ${appliedOn}`:""}`;
 const stop=(event:React.PointerEvent)=>event.stopPropagation();
 const move=(target:BoardColumn)=>()=>run(()=>moveTo(app,target),movedText(name,target));
 return <article className="card" ref={drag.setNodeRef} {...drag.listeners} {...drag.attributes} style={{opacity:drag.isDragging?.55:1}}>
  <strong>{app.title||"Job"}</strong><p>{app.company}</p><span className="when">{when}</span>
  <span className="card-actions">
   <button className="small" disabled={!url} title={url?undefined:"This listing has no web address"} onPointerDown={stop}
    onClick={()=>url&&openUrl(url).catch(error=>onStatus(String(error)))}>Open website</button>
   {column==="planned"&&<>
    <button className="small primary" disabled={busy} title="Record that you have applied. Nothing is opened or sent." onPointerDown={stop} onClick={move("applied")}>Mark applied</button>
    <button className="small quiet" disabled={busy} onPointerDown={stop} onClick={remove}>Unsave</button></>}
   {column==="applied"&&<>
    <button className="small" disabled={busy} title="The application is over: rejected, or no longer pursued" onPointerDown={stop} onClick={move("rejected")}>Mark rejected</button>
    <button className="small quiet" disabled={busy} onPointerDown={stop} onClick={move("planned")}>Back to saved</button></>}
   {column==="rejected"&&<>
    <button className="small" disabled={busy} title="Move it back to Applied" onPointerDown={stop} onClick={move("applied")}>Reopen</button>
    <button className="small quiet" disabled={busy} onPointerDown={stop} onClick={remove}>Remove</button></>}
  </span></article>}
function StageColumn({stage,count,children}:{stage:BoardColumn;count:number;children:React.ReactNode}){const drop=useDroppable({id:stage});
 return <section ref={drop.setNodeRef} className={drop.isOver?"drop-active":""}>
  <h3><span className="name">{columnLabel(stage)}</span><span className="count">{count}</span></h3>
  {count?children:<div className="empty">{stage==="planned"?"Nothing saved":stage==="applied"?"Nothing applied to yet":"Nothing rejected"}</div>}</section>}
function Applications(){const qc=useQueryClient();const {data:apps=[]}=useQuery({queryKey:["apps"],queryFn:api.apps});const [status,setStatus]=useState("");const [search,setSearch]=useState("");
 const refresh=()=>{qc.invalidateQueries({queryKey:["apps"]});refreshJobs(qc)};
 const onDragEnd=(event:DragEndEvent)=>{const app=event.active.data.current?.application as Application|undefined;const target=event.over?.id;
  if(!app||typeof target!=="string"||!(BOARD_COLUMNS as readonly string[]).includes(target)||target===boardColumn(app.currentStage))return;
  const column=target as BoardColumn,name=app.title||"this job";
  moveTo(app,column).then(()=>{setStatus(movedText(name,column));refresh()}).catch(error=>setStatus(String(error)))};
 const words=search.toLowerCase().trim().split(/\s+/).filter(Boolean);
 const visible=apps.filter(app=>words.every(word=>`${app.title??""} ${app.company??""}`.toLowerCase().includes(word)));
 const counts=Object.fromEntries(BOARD_COLUMNS.map(column=>[column,apps.filter(app=>boardColumn(app.currentStage)===column).length])) as Record<BoardColumn,number>;
 return <div className="view">
  <div className="toolbar"><header><div><h2>Applications</h2><p>Save a job on the Jobs page and it lands here. Mark it applied once you have, and rejected when it is over — or drag the card across.</p></div><span className="count">{counts.planned} saved · {counts.applied} applied · {counts.rejected} rejected</span></header>
   {apps.length>0&&<div className="filters"><label className="search"><span>Search applications</span><input type="search" value={search} onChange={event=>setSearch(event.target.value)} placeholder="Title or company"/></label></div>}
   {status&&<output className="status">{status}</output>}</div>
  <div className="scroll" style={{display:"block"}}>
   {!apps.length?<div className="empty"><strong>Nothing saved yet</strong><span>Press Save or Apply on a job and it lands here.</span></div>
    :!visible.length?<div className="empty"><strong>No application matches</strong><button onClick={()=>setSearch("")}>Clear search</button></div>
    :<DndContext onDragEnd={onDragEnd}><div className="board">{BOARD_COLUMNS.map(stage=>{const cards=visible.filter(a=>boardColumn(a.currentStage)===stage);
     return <StageColumn stage={stage} count={cards.length} key={stage}>{cards.map(a=><AppCard app={a} onStatus={setStatus} refresh={refresh} key={a.id}/>)}</StageColumn>})}</div></DndContext>}
  </div>
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
   <div className="summary-row"><div><span>Database</span><strong>{data?`Schema ${data.schemaVersion}`:"Loading"}</strong><small>{data?.dbPath||"Opening local data"}</small></div><div className={attention.length?"bad":""}><span>Sources switched on</span><strong>{active} of {total}</strong><small>{attention.length?`${attention.length} need attention`:"No source failures"}</small></div><div><span>Listings stored</span><strong>{data?.jobCount.toLocaleString()??"—"}</strong><small>{data?`${data.sourceCount} sources in database`:"Loading"}</small></div><div><span>Sidecar</span><strong>{data?.sidecarActive?"Running":"Idle"}</strong><small>starts only on test or update</small></div>{restore&&<div><span>Restore</span><strong>{restore.applied?"Applied":"Ready"}</strong><small>{restore.message}</small></div>}</div>
   {!!attention.length&&<><div className="rule"><h3>Needs attention</h3><span className="count">{attention.length} source{attention.length===1?"":"s"}</span></div><div className="attn">{attention.map(source=><AttentionRow key={source.id} source={source}/>)}</div></>}
   <ScrapePerformance/>
   <ActivityLog/>
   <p className="hint">City and country data © <a href="https://www.geonames.org" target="_blank" rel="noreferrer">GeoNames</a>, CC BY 4.0. Bundled with the app; nothing is looked up online.</p>
   {data&&<details className="fold" style={{marginTop:34}}><summary>Raw report (JSON)</summary><pre>{JSON.stringify(data,null,2)}</pre></details>}
  </div>
 </div>}
// A test that said nothing looked like a button that did nothing: the outcome replaces the reason while it is fresh.
function AttentionRow({source}:{source:Source}){
 const qc=useQueryClient();const [busy,setBusy]=useState(false);const [result,setResult]=useState("");
 const run=()=>{setBusy(true);setResult("");api.test(source).then(events=>setResult(describeOutcome(events,source.name)))
  .catch(()=>setResult(`${source.name}: ${describeFailure()} See Recent activity below.`))
  .finally(()=>{setBusy(false);qc.invalidateQueries({queryKey:["app-logs"]})})};
 return <div><strong>{source.name}</strong><span className="why" aria-live="polite">{busy?"Testing…":result||source.disabledReason}</span>
  <span className="actions"><button className="small" disabled={busy} onClick={run}>{busy?"Testing…":"Test now"}</button></span></div>}
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

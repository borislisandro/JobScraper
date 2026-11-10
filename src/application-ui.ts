// Applications are three columns: Saved (the database calls it "planned"), Applied, and Rejected — the
// end of the road, which is what keeps the Applied column to the applications still in play. Older rows
// may carry an in-between stage such as "interviewing" from before the board was simplified; those
// count as Applied. A withdrawn application ended too, so it sits with the rejected ones.
export const BOARD_COLUMNS=["planned","applied","rejected"] as const;
export type BoardColumn=typeof BOARD_COLUMNS[number];
/** True once the job has actually been applied to, rather than only saved for later. */
export const isApplied=(stage?:string)=>!!stage&&stage!=="planned";
/** True once the application is over, whichever side ended it. */
export const isEnded=(stage?:string)=>stage==="rejected"||stage==="withdrawn";
// Every stage the backend records. The board keeps three columns; the stages past Applied are chosen in
// the application panel and named on the card, so an interview in progress no longer reads as "Applied".
export const STAGES=["planned","applied","screening","interviewing","offer","accepted","rejected","withdrawn"] as const;
const STAGE_NAMES:Record<string,string>={planned:"Saved",applied:"Applied",screening:"Screening",interviewing:"Interviewing",offer:"Offer",accepted:"Accepted",rejected:"Rejected",withdrawn:"Withdrawn"};
export const stageLabel=(stage?:string)=>!stage?"":STAGE_NAMES[stage]??(isApplied(stage)?"Applied":"Saved");
/** Which board column a stored stage belongs in. */
export const boardColumn=(stage:string):BoardColumn=>isEnded(stage)?"rejected":isApplied(stage)?"applied":"planned";
export const columnLabel=(column:BoardColumn)=>column==="planned"?"Saved":column==="applied"?"Applied":"Rejected";
/** Where "Open website" goes: the listing itself, falling back to its apply link. */
export const websiteUrl=(app:{canonicalUrl?:string;applyUrl?:string})=>app.canonicalUrl||app.applyUrl;
/** Whole days from one instant to another, rounded towards the nearer day. */
export const daysBetween=(from:string|number|Date,to:string|number|Date)=>Math.round((new Date(to).getTime()-new Date(from).getTime())/86_400_000);
/** A follow-up is due once the reminder of an application still waiting in Applied has come due. */
export const followUpDue=(app:{currentStage:string;followUpAt?:string|null},now=Date.now())=>app.currentStage==="applied"&&!!app.followUpAt&&new Date(app.followUpAt).getTime()<=now;
/** The badge a saved job carries once its board stops listing it. */
export const listingGone=(availability?:string|null)=>availability==="closed"?"Closed":availability==="possibly_closed"?"May be gone":null;
export type SavedOrder="saved"|"posted"|"closing";
const time=(value?:string|null)=>{const at=value?new Date(/^\d{4}-\d{2}-\d{2}$/.test(value)?`${value}T00:00:00`:value).getTime():NaN;return Number.isNaN(at)?null:at};
/** Orders cards: newest saved, newest posted, or closing soonest. Undated cards go last either way. */
export function orderApplications<T extends {createdAt?:string;jobPostedAt?:string|null;jobClosingAt?:string|null}>(apps:T[],order:SavedOrder):T[]{
 const key=(app:T)=>order==="posted"?time(app.jobPostedAt):order==="closing"?time(app.jobClosingAt):time(app.createdAt);
 return [...apps].sort((a,b)=>{const x=key(a),y=key(b);if(x===null||y===null)return x===null?(y===null?0:1):-1;return order==="closing"?x-y:y-x});
}
/** One line per timeline event, in the words the board uses. */
export function describeEvent(event:{eventType:string;fromStage?:string;toStage?:string;reason?:string;payloadJson:string}){
 let payload:Record<string,unknown>={};try{payload=JSON.parse(event.payloadJson||"{}")}catch{/* an unreadable payload still has a type */}
 const reason=event.reason?` — ${event.reason}`:"";
 switch(event.eventType){
  case "created":return "Saved";
  case "stage_changed":return `${stageLabel(event.toStage)||"Stage changed"}${reason}`;
  case "apply_confirmation":return payload.decision==="yes"?"Confirmed applied":"Apply not confirmed";
  case "note_saved":return "Note saved";
  case "note_deleted":return "Note deleted";
  case "document_attached":return `Attached ${String(payload.filename??"a document")}`;
  case "interview_scheduled":return `Interview scheduled: ${String(payload.stage??"")} ${payload.scheduledAt?new Date(String(payload.scheduledAt)).toLocaleString([],{dateStyle:"medium",timeStyle:"short"}):""}`.trim();
  case "interview_deleted":return "Interview removed";
  case "ghost_threshold_changed":return `Follow-up set to ${String(payload.days??"?")} days after applying`;
  case "details_updated":return "Recruiter details updated";
  default:return event.eventType.replaceAll("_"," ");
 }
}

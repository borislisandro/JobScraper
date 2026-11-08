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
export const stageLabel=(stage?:string)=>!stage?"":stage==="rejected"?"Rejected":stage==="withdrawn"?"Withdrawn":isApplied(stage)?"Applied":"Saved";
/** Which board column a stored stage belongs in. */
export const boardColumn=(stage:string):BoardColumn=>isEnded(stage)?"rejected":isApplied(stage)?"applied":"planned";
export const columnLabel=(column:BoardColumn)=>column==="planned"?"Saved":column==="applied"?"Applied":"Rejected";
/** Where "Open website" goes: the listing itself, falling back to its apply link. */
export const websiteUrl=(app:{canonicalUrl?:string;applyUrl?:string})=>app.canonicalUrl||app.applyUrl;

// Applications are two states and nothing more: Saved (the database calls it "planned") and
// Applied. Older rows may still carry a later stage such as "interviewing" from before the board
// was simplified; those count as Applied, so they land in that column and read that way everywhere.
export const BOARD_COLUMNS=["planned","applied"] as const;
/** True once the job has actually been applied to, rather than only saved for later. */
export const isApplied=(stage?:string)=>!!stage&&stage!=="planned";
export const stageLabel=(stage?:string)=>!stage?"":isApplied(stage)?"Applied":"Saved";
/** Which board column a stored stage belongs in. */
export const boardColumn=(stage:string)=>isApplied(stage)?"applied":"planned";
/** Where "Open website" goes: the listing itself, falling back to its apply link. */
export const websiteUrl=(app:{canonicalUrl?:string;applyUrl?:string})=>app.canonicalUrl||app.applyUrl;

export type ApplyDecision = "yes" | "no" | "not_yet";
export function applyDecisionOutcome(decision: ApplyDecision) { return { recordsApplied: decision === "yes", eventType: "apply_confirmation" as const, decision }; }
export function stageMoveAllowed(from: string, to: string) { return from !== to && ["planned","applied","screening","interviewing","offer","accepted","rejected","withdrawn"].includes(to); }
/** UI event reducer: same focus event must not reopen an already-visible dialog. */
export function acceptApplyPrompt(currentAttemptId: string | undefined, incomingAttemptId: string) { return currentAttemptId !== incomingAttemptId; }
export function applyPromptRemainsPending(decision: ApplyDecision) { return decision === "not_yet"; }

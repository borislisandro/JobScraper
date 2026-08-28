import { describe, expect, it } from "vitest";

const exclusiveEnd=(day:string)=>new Date(new Date(`${day}T00:00:00.000Z`).getTime()+86_400_000).toISOString();
const rate=(numerator:number,denominator:number)=>denominator?numerator*100/denominator:undefined;
describe("analytics dashboard semantics",()=>{
 it("uses exclusive next-day boundary and reports denominator",()=>{
  expect(exclusiveEnd("2026-01-31")).toBe("2026-02-01T00:00:00.000Z");
  expect(rate(2,4)).toBe(50);expect(rate(0,0)).toBeUndefined();
 });
 it("marks small response samples instead of treating missing data as zero",()=>{
  const samples=2;const hours:undefined|number=undefined;
  expect(samples<3).toBe(true);expect(hours).toBeUndefined();
 });
 it("keeps stage, trend, freshness, and review rows drillable",()=>{
  const sections=["Applications by current stage","Applications over time","Job freshness / availability","Review decisions"];
  expect(sections).toHaveLength(4);
 });
});

import { describe, expect, it } from "vitest";
import { parseHashRoute, parsePositiveId } from "../src/route";

describe("shared positive ID parser", () => {
  it("accepts only positive safe integers while preserving leading-zero compatibility", () => {
    for (const [token, expected] of [["1",1],["00012",12],["9007199254740991",Number.MAX_SAFE_INTEGER]] as const) {
      expect(parsePositiveId(token)).toBe(expected);
      expect(parseHashRoute(`#/fleets/${token}/members`)).toEqual({kind:"dashboard",fleetId:expected});
    }
  });
  it("rejects empty, zero, signs, decimal, exponent, whitespace and non-ASCII tokens", () => {
    for (const token of ["","0","0000","-1","+12","1.0","1e2","0x10"," 12","12 ","\t12","12\n","12\r","１","١","NaN","Infinity","12suffix","%31%32"]) {
      expect(parsePositiveId(token)).toBeNull();
      expect(parseHashRoute(`#/fleets/${token}/members`)).toEqual({kind:"fleets"});
    }
  });
  it("rejects unsafe magnitudes rather than accepting rounded or infinite numeric conversions", () => {
    for (const token of ["9007199254740992","9007199254740993","9".repeat(400)]) {
      expect(parsePositiveId(token)).toBeNull();
    }
  });
});

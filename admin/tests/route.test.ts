import { describe, expect, it } from "vitest";
import { parseHashRoute, parsePositiveId } from "../src/route";

describe("whole hash route and numeric ID validation", () => {
  it("accepts both hash spellings, leading zero IDs, and the largest safe integer", () => {
    for (const hash of ["#/fleets/007/members","#fleets/7/members"]) {
      expect(parseHashRoute(hash)).toEqual({kind:"dashboard",fleetId:7});
    }
    expect(parseHashRoute(`#/fleets/${Number.MAX_SAFE_INTEGER}/members`)).toEqual({kind:"dashboard",fleetId:Number.MAX_SAFE_INTEGER});
    const route=parseHashRoute("#/fleets/007/members/00042");
    expect(route.kind).toBe("dashboard");
    if (route.kind !== "dashboard") throw new Error("valid dashboard");
    expect(route.fleetId).toBe(7); expect(Number(route.memberId)).toBe(42);
  });
  it("rejects every invalid fleet lexical form without Number coercion", () => {
    for (const id of ["", "0", "00", "-1", "+1", "1e2", "1.0", " 1", "1 ", "%31", "１", "١", "Infinity", "NaN", "9007199254740992"]) {
      expect(parseHashRoute(`#/fleets/${id}/members`)).toEqual({kind:"fleets"});
    }
  });
  it("does not partially match unknown paths or suffixes", () => {
    for (const hash of ["", "#", "#/fleets", "#/unknown", "#/fleets/7", "#/fleets/7/members/42/extra", "#/fleets/7/members?x=1"]) {
      expect(parseHashRoute(hash)).toEqual({kind:"fleets"});
    }
  });
  it("keeps the valid fleet when the member token needs dashboard validation", () => {
    for (const member of ["0","+42","4.2e1"," 42","9007199254740992","９"]) {
      const route=parseHashRoute(`#/fleets/7/members/${member}`);
      expect(route.kind).toBe("dashboard");
      if (route.kind === "dashboard") expect(route.fleetId).toBe(7);
    }
  });
});

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

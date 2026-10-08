// Runs a conflict draft that real git produced through the parser.
// Driven by scripts/verify-conflict.ps1, which builds the fixture repo.
//
// This is the "same result as hand-rolled git" acceptance check from section 7.13 --
// the parser is only trustworthy if it agrees with the markers git actually writes.
import { readFileSync } from "node:fs";
import { parseConflict, assemble } from "../src/lib/conflict.ts";

const text = readFileSync(process.argv[2], "utf8");
const parsed = parseConflict(text);
if (!parsed) {
  console.error("FAILED: could not parse the draft git produced");
  process.exit(1);
}

const pick = (kind) => assemble(parsed, parsed.hunk.map(() => kind));
const theirs = pick("theirs").replace(/\r/g, "");

console.log("hunks:", parsed.hunk.length);
console.log("clean segments:", JSON.stringify(parsed.clean));
console.log("--- all ours ---\n" + pick("ours"));
console.log("--- all theirs ---\n" + theirs);
console.log("--- all base ---\n" + pick("base"));

// Taking "theirs" everywhere must land exactly on side's version of the file.
const expected = "CHANGED_SIDE\nline1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nCHANGED_SIDE_END\n";
if (theirs !== expected) {
  console.error(
    `FAILED: all-theirs does not match side\n  got:      ${JSON.stringify(theirs)}\n  expected: ${JSON.stringify(expected)}`,
  );
  process.exit(1);
}

// Ours differs in exactly two lines. Anything else means the hunks drifted or
// a clean segment moved -- both silently produce a plausible-looking wrong file.
const ours = pick("ours").replace(/\r/g, "");
const changed = ours.split("\n").filter((line, i) => line !== expected.split("\n")[i]).length;
console.log("lines where all-ours differs from all-theirs:", changed);
if (parsed.hunk.length !== 2 || changed !== 2) {
  console.error("FAILED: expected 2 hunks and 2 differing lines");
  process.exit(1);
}
console.log("\nOK: parser agrees with what git actually wrote");

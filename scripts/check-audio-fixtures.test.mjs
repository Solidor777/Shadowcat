import { renameSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { makeRepo } from "./lib/temp-repo.mjs";
import {
  fixtureListingFailures,
  levelFailures,
  main,
  parseIntegratedLufs,
  parseMaxVolume,
  byteEqualityEnforceable,
} from "./check-audio-fixtures.mjs";

describe("audio fixture level parsing", () => {
  it("reads volumedetect's max_volume", () => {
    expect(parseMaxVolume("[Parsed_volumedetect_0 @ 0x1] max_volume: -11.9 dB\n")).toBe(-11.9);
    expect(parseMaxVolume("nothing")).toBeNull();
  });

  it("reads the integrated loudness from the summary only", () => {
    const stderr =
      "[Parsed_ebur128_0 @ 0x1] t: 1 TARGET:-23 LUFS M: -50 S: -40 I: -99.0 LUFS LRA: 0.0 LU\n" +
      "[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -22.9 LUFS\n    Threshold: -33.0 LUFS\n";
    expect(parseIntegratedLufs(stderr)).toBe(-22.9);
    expect(parseIntegratedLufs("none")).toBeNull();
  });
});

describe("levelFailures", () => {
  it("passes a quiet in-band track", () => {
    expect(levelFailures(-11.9, -22.9)).toEqual([]);
  });

  it("fails at a peak over the bound, at each loudness edge, and on a missing reading", () => {
    expect(levelFailures(-10.9, -22.9)).toHaveLength(1);
    expect(levelFailures(-12, -19.9)).toHaveLength(1);
    expect(levelFailures(-12, -26.1)).toHaveLength(1);
    expect(levelFailures(-11, -20)).toEqual([]);
    expect(levelFailures(-11, -26)).toEqual([]);
    expect(levelFailures(null, null)).toHaveLength(2);
  });
});

// The fixture listing compares the directory with the manifest in two directions: a manifest track
// must have a TRACKED file (so a never-added file cannot pass here and fail on a clean checkout),
// and an extra `.ogg` is found in the untracked-inclusive corpus (so a stray is caught before
// `git add`). Only direct children of the directory count.
describe("fixtureListingFailures", () => {
  const DIR = "fx";
  const quiet = () => {};
  /** Lists `fx/` of a scratch repository holding `files`, with `tracked` indexed. */
  const list = (files, tracked, tracks = ["a", "b"]) => {
    const cwd = makeRepo(files, tracked);
    return { cwd, failures: fixtureListingFailures({ cwd, dir: join(cwd, DIR), tracks, log: quiet }) };
  };

  it("passes when every manifest track has a tracked file and nothing else is present", () => {
    expect(list({ "fx/a.ogg": "", "fx/b.ogg": "" }, ["fx/a.ogg", "fx/b.ogg"]).failures).toEqual([]);
  });

  it("fails a manifest track whose file exists but was never added to git", () => {
    const { failures } = list({ "fx/a.ogg": "", "fx/b.ogg": "" }, ["fx/a.ogg"]);
    expect(failures).toEqual(["manifest track file b.ogg is not a tracked file present in fx"]);
  });

  it("fails an untracked stray .ogg that is not a manifest track", () => {
    const { failures } = list({ "fx/a.ogg": "", "fx/b.ogg": "", "fx/stray.ogg": "" }, ["fx/a.ogg", "fx/b.ogg"]);
    expect(failures).toEqual(["fx/stray.ogg is not a manifest track"]);
  });

  it("ignores a stray .ogg under an ignored path", () => {
    const { failures } = list(
      { ".gitignore": "fx/stray.ogg\n", "fx/a.ogg": "", "fx/b.ogg": "", "fx/stray.ogg": "" },
      [".gitignore", "fx/a.ogg", "fx/b.ogg"],
    );
    expect(failures).toEqual([]);
  });

  it("fails a tracked track file deleted in the working tree without throwing", () => {
    const { cwd } = list({ "fx/a.ogg": "", "fx/b.ogg": "" }, ["fx/a.ogg", "fx/b.ogg"]);
    renameSync(join(cwd, DIR, "b.ogg"), join(cwd, "b-aside.txt"));
    const failures = fixtureListingFailures({ cwd, dir: join(cwd, DIR), tracks: ["a", "b"], log: quiet });
    expect(failures).toEqual(["manifest track file b.ogg is not a tracked file present in fx"]);
  });

  it("counts only direct children of the directory", () => {
    const { failures } = list(
      { "fx/a.ogg": "", "fx/b.ogg": "", "fx/sub/extra.ogg": "" },
      ["fx/a.ogg", "fx/b.ogg"],
    );
    expect(failures).toEqual([]);
  });

  it("names a directory outside the repository instead of blaming the manifest", () => {
    const cwd = makeRepo({ "fx/a.ogg": "" });
    const failures = fixtureListingFailures({ cwd, dir: join(tmpdir(), "elsewhere"), tracks: ["a"], log: quiet });
    expect(failures).toHaveLength(1);
    expect(failures[0]).toContain("is outside the repository");
  });

  // A library function prints nothing unless its caller passes a log.
  it("prints nothing when no log is passed", () => {
    const cwd = makeRepo({ "fx/a.ogg": "" }, ["fx/a.ogg"]);
    const printed = [];
    const original = console.log;
    console.log = (line) => printed.push(line);
    try {
      fixtureListingFailures({ cwd, dir: join(cwd, DIR), tracks: ["a"] });
    } finally {
      console.log = original;
    }
    expect(printed).toEqual([]);
  });
});

// The command lists the fixtures before it looks for tools and passes `console.log`, so the corpus
// line is on stdout even when every tool is missing.
describe("main", () => {
  it("prints the corpus line on stdout and skips cleanly when the tools are missing", () => {
    const printed = [];
    const original = console.log;
    console.log = (line) => printed.push(line);
    let code;
    try {
      code = main([], () => ["ffmpeg"]);
    } finally {
      console.log = original;
    }
    expect(code).toBe(0);
    expect(printed[0]).toMatch(/^corpus lint:audio-fixtures: \d+ files$/);
    expect(printed[1]).toContain("SKIPPED");
  });
});

describe("byteEqualityEnforceable", () => {
  const manifest = { ffmpeg_version: "8.1.1", numpy_version: "2.1.2" };

  it("holds only when both recorded versions match", () => {
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "8.1.1", numpy_version: "2.1.2" })).toBe(true);
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "6.1.1", numpy_version: "2.1.2" })).toBe(false);
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "8.1.1", numpy_version: "1.26.4" })).toBe(false);
  });
});

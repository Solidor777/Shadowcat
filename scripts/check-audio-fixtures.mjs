// Regenerates the long-audio e2e fixtures and compares them with the committed files.
//
// The committed tracks under `src/client/shell/e2e-fixtures/audio/` come from
// `scripts/gen-audio-fixtures.py`; this check proves the generator still produces them, so CI
// needs no ffmpeg at e2e time and a stale or hand-edited file fails here.
//
// Two tiers, because a Vorbis encode is byte-reproducible only for one ffmpeg build:
//   Tier 1 (always): per track, the committed AND the regenerated file have duration exactly
//     `DURATION_SECS`, the manifest's sample rate and 2 channels; the regenerated manifest sequence
//     equals the committed one; `verify-audio-markers.py` recovers the manifest sequence from both
//     files with 0 mismatches; the sample peak is at most `PEAK_MAX_DB` and the integrated loudness
//     lies in `LOUDNESS_MIN_LUFS..LOUDNESS_MAX_LUFS` (the "not painful" bound).
//   Tier 2 (byte equality of every `.ogg`): enforced only when the installed ffmpeg and NumPy
//     versions equal the committed manifest's; otherwise the two version sets are printed and the
//     tier never fails.
//
// `--require-tools` makes a missing ffmpeg, libvorbis encoder, python or NumPy/SciPy FATAL. Without
// it a missing tool prints SKIPPED and exits 0, which is only for a developer machine: a runner that
// skips proves nothing, so CI always passes the flag.
//
// Cross-platform: node:child_process with argument arrays, node:path/node:os only.

import { Buffer } from "node:buffer";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import process from "node:process";
import { URL, fileURLToPath } from "node:url";
import { isDirectEntry } from "./lib/is-main.mjs";

const HERE = fileURLToPath(new URL(".", import.meta.url));
export const FIXTURE_DIR = resolve(HERE, "../src/client/shell/e2e-fixtures/audio");
export const GENERATOR = resolve(HERE, "gen-audio-fixtures.py");
export const VERIFIER = resolve(HERE, "verify-audio-markers.py");
export const DURATION_SECS = 1200;
export const CHANNELS = 2;
// Not-painful bound: Vorbis peak and integrated loudness of the generated beds.
export const PEAK_MAX_DB = -11;
export const LOUDNESS_MIN_LUFS = -26;
export const LOUDNESS_MAX_LUFS = -20;

/** `max_volume` in dBFS from ffmpeg's `volumedetect` stderr, or null when absent. */
export function parseMaxVolume(stderr) {
  const m = stderr.match(/max_volume:\s*(-?\d+(?:\.\d+)?)\s*dB/);
  return m ? Number(m[1]) : null;
}

/** Integrated loudness in LUFS from ffmpeg's `ebur128` summary stderr, or null when absent. */
export function parseIntegratedLufs(stderr) {
  const summary = stderr.split("Summary:").pop() ?? "";
  const m = summary.match(/I:\s*(-?\d+(?:\.\d+)?)\s*LUFS/);
  return m ? Number(m[1]) : null;
}

/** Whether a measured peak and loudness sit inside the not-painful bands; returns the failures. */
export function levelFailures(peakDb, lufs) {
  const out = [];
  if (peakDb === null || !(peakDb <= PEAK_MAX_DB)) {
    out.push(`peak ${peakDb} dBFS exceeds ${PEAK_MAX_DB}`);
  }
  if (lufs === null || !(lufs >= LOUDNESS_MIN_LUFS && lufs <= LOUDNESS_MAX_LUFS)) {
    out.push(`integrated loudness ${lufs} LUFS outside ${LOUDNESS_MIN_LUFS}..${LOUDNESS_MAX_LUFS}`);
  }
  return out;
}

/** Whether tier 2 (byte equality) is enforceable: both recorded versions equal the installed ones. */
export function tier2Enforceable(manifest, installed) {
  return (
    manifest.ffmpeg_version === installed.ffmpeg_version &&
    manifest.numpy_version === installed.numpy_version
  );
}

function run(cmd, args, opts = {}) {
  return spawnSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 28, ...opts });
}

/** First python interpreter that imports NumPy and SciPy, or null. */
export function findPython() {
  for (const cmd of ["python3", "python"]) {
    const r = run(cmd, ["-c", "import numpy, scipy"]);
    if (r.status === 0) return cmd;
  }
  return null;
}

/** Missing-tool reasons: ffmpeg, its libvorbis encoder, ffprobe, python with NumPy/SciPy. */
export function missingTools() {
  const missing = [];
  const ff = run("ffmpeg", ["-version"]);
  if (ff.status !== 0) missing.push("ffmpeg");
  else if (!/libvorbis/.test(run("ffmpeg", ["-hide_banner", "-encoders"]).stdout ?? "")) {
    missing.push("ffmpeg libvorbis encoder");
  }
  if (run("ffprobe", ["-version"]).status !== 0) missing.push("ffprobe");
  if (findPython() === null) missing.push("python3 with numpy and scipy");
  return missing;
}

function probe(file) {
  const r = run("ffprobe", [
    "-v", "error", "-select_streams", "a:0",
    "-show_entries", "stream=sample_rate,channels,duration_ts,time_base",
    "-of", "json", file,
  ]);
  if (r.status !== 0) throw new Error(`ffprobe failed on ${file}: ${r.stderr}`);
  const s = JSON.parse(r.stdout).streams[0];
  const [num, den] = s.time_base.split("/").map(Number);
  return {
    sampleRate: Number(s.sample_rate),
    channels: Number(s.channels),
    seconds: (Number(s.duration_ts) * num) / den,
  };
}

function levels(file) {
  const vol = run("ffmpeg", ["-hide_banner", "-nostats", "-i", file, "-af", "volumedetect", "-f", "null", "-"]);
  const lufs = run("ffmpeg", ["-hide_banner", "-nostats", "-i", file, "-af", "ebur128", "-f", "null", "-"]);
  return { peak: parseMaxVolume(vol.stderr), lufs: parseIntegratedLufs(lufs.stderr) };
}

function markerMismatches(python, file, manifestPath, track) {
  const r = run(python, [VERIFIER, file, manifestPath, track]);
  const m = (r.stdout ?? "").match(/MARKERS slots=(\d+) mismatches=(\d+)/);
  if (r.status !== 0 || !m) return { ok: false, detail: `verifier failed: ${r.stderr}` };
  return { ok: Number(m[2]) === 0 && Number(m[1]) === DURATION_SECS / 10, detail: m[0] };
}

/** Runs both tiers; returns `{ failures: string[], lines: string[] }`. */
export function compare({ python, committedDir = FIXTURE_DIR }) {
  const failures = [];
  const lines = [];
  const regenDir = mkdtempSync(join(tmpdir(), "shadowcat-audio-fixtures-"));
  const gen = run(python, [GENERATOR, regenDir]);
  if (gen.status !== 0) {
    return { failures: [`generator failed: ${gen.stderr}`], lines };
  }
  const committedManifestPath = join(committedDir, "manifest.json");
  const regenManifestPath = join(regenDir, "manifest.json");
  const committed = JSON.parse(readFileSync(committedManifestPath, "utf8"));
  const regen = JSON.parse(readFileSync(regenManifestPath, "utf8"));
  const tracks = Object.keys(committed.tracks).sort();
  const onDisk = readdirSync(committedDir).filter((n) => n.endsWith(".ogg")).sort();
  if (JSON.stringify(onDisk) !== JSON.stringify(tracks.map((t) => `${t}.ogg`))) {
    failures.push(`committed .ogg files ${onDisk} differ from the manifest tracks ${tracks}`);
  }
  for (const track of tracks) {
    const rt = regen.tracks[track];
    if (!rt || JSON.stringify(rt.sequence) !== JSON.stringify(committed.tracks[track].sequence)) {
      failures.push(`${track}: regenerated sequence differs from the committed manifest`);
    }
    for (const [label, dir, manifestPath] of [
      ["committed", committedDir, committedManifestPath],
      ["regenerated", regenDir, regenManifestPath],
    ]) {
      const file = join(dir, `${track}.ogg`);
      const p = probe(file);
      if (p.seconds !== DURATION_SECS || p.sampleRate !== committed.sample_rate || p.channels !== CHANNELS) {
        failures.push(`${track} (${label}): ${p.seconds} s, ${p.sampleRate} Hz, ${p.channels} ch`);
      }
      const mk = markerMismatches(python, file, manifestPath, track);
      lines.push(`${track} (${label}) ${mk.detail}`);
      if (!mk.ok) failures.push(`${track} (${label}): markers ${mk.detail}`);
      const lv = levels(file);
      lines.push(`${track} (${label}) peak ${lv.peak} dBFS, integrated ${lv.lufs} LUFS`);
      for (const f of levelFailures(lv.peak, lv.lufs)) failures.push(`${track} (${label}): ${f}`);
    }
  }
  const installed = {
    ffmpeg_version: regen.ffmpeg_version,
    numpy_version: regen.numpy_version,
  };
  if (tier2Enforceable(committed, installed)) {
    for (const track of tracks) {
      const a = readFileSync(join(committedDir, `${track}.ogg`));
      const b = readFileSync(join(regenDir, `${track}.ogg`));
      if (Buffer.compare(a, b) !== 0) failures.push(`${track}: regenerated bytes differ from the committed file`);
    }
    lines.push("tier 2 (byte equality): enforced, versions match the manifest");
  } else {
    lines.push(
      `tier 2 (byte equality): NOT enforced; manifest ffmpeg ${committed.ffmpeg_version} numpy ${committed.numpy_version}, ` +
        `installed ffmpeg ${installed.ffmpeg_version} numpy ${installed.numpy_version}`,
    );
  }
  return { failures, lines };
}

function main(argv) {
  const requireTools = argv.includes("--require-tools");
  const missing = missingTools();
  if (missing.length > 0) {
    const msg = `audio fixtures: missing tools: ${missing.join(", ")}`;
    if (requireTools) {
      console.error(`${msg} (FATAL under --require-tools)`);
      return 1;
    }
    console.log(`${msg}; SKIPPED (pass --require-tools to make this fatal)`);
    return 0;
  }
  const { failures, lines } = compare({ python: findPython() });
  for (const l of lines) console.log(l);
  if (failures.length > 0) {
    for (const f of failures) console.error(`FAIL ${f}`);
    return 1;
  }
  console.log("audio fixtures: ok");
  return 0;
}

if (isDirectEntry(import.meta.url)) {
  process.exit(main(process.argv.slice(2)));
}

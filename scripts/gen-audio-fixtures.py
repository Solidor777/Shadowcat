#!/usr/bin/env python3
"""Reproducible generator for the committed long-audio e2e fixtures: three soft Ogg Vorbis tracks.

Position code: every SLOT_SECS slot holds one bass sine (the marker) at MARKER_BASE_HZ * 2^(r/12),
r = sequence[slot] in 0..11. Marker band is 100..215 Hz; no other component lies in it (pad tones
start at 4 * marker, the pulse thump is 55 Hz, noise is low-passed but its band energy is far below
the marker). A window of FOUR consecutive slots identifies the position uniquely (verified below).

Usage: gen-audio-fixtures.py OUTDIR [--secs N]
Needs: python3, numpy, scipy, ffmpeg with libvorbis. The position sequence comes from an in-script
integer PRNG (`xorshift64`), so it never depends on a NumPy version; NumPy supplies only the noise bed
and the sine synthesis. The Vorbis encode is byte-reproducible only for the same ffmpeg build, which
`manifest.json` records beside the NumPy version. `-fflags +bitexact -flags:a +bitexact` removes the
Ogg muxer's random stream serial, the one source of run-to-run difference on a fixed build.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np
from scipy import signal

SR = 44100
SLOT_SECS = 10
XFADE = 2.0
MARKER_BASE_HZ = 110.0
TRACKS = {
    "ambient-bed": {"seed": 11, "minor": False, "pulse": 0.0, "noise_lp": 700.0, "noise_amp": 0.020, "pad_amp": 0.060},
    "cave-atmosphere": {"seed": 22, "minor": True, "pulse": 0.0, "noise_lp": 380.0, "noise_amp": 0.060, "pad_amp": 0.030},
    "battle-bed": {"seed": 33, "minor": True, "pulse": 0.10, "noise_lp": 900.0, "noise_amp": 0.018, "pad_amp": 0.050},
}
PEAK_TARGET = 0.25  # -12 dBFS


MASK = (1 << 64) - 1


def xorshift64(state):
    """Platform-independent integer PRNG (Marsaglia xorshift64); returns the next state."""
    state ^= (state << 13) & MASK
    state ^= state >> 7
    state ^= (state << 17) & MASK
    return state


def sequence(seed, n):
    state = (seed * 0x9E3779B97F4A7C15 + 1) & MASK
    while True:
        seq = []
        while len(seq) < n:
            state = xorshift64(state)
            c = (state >> 33) % 12
            if not seq or c != seq[-1]:
                seq.append(int(c))
        grams = {tuple(seq[i:i + 4]) for i in range(n - 3)}
        if len(grams) == n - 3:
            return seq


def fade_env(n, fade):
    env = np.ones(n)
    f = int(fade * SR)
    ramp = 0.5 - 0.5 * np.cos(np.linspace(0, np.pi, f))
    env[:f] = ramp
    env[-f:] = ramp[::-1]
    return env


def render_slot(r, cfg, rng_noise_unused, t0):
    n = int((SLOT_SECS + 2 * XFADE) * SR)
    t = (np.arange(n) / SR) + t0
    fm = MARKER_BASE_HZ * 2 ** (r / 12)
    marker = 0.10 * np.sin(2 * np.pi * fm * t)
    third = 3 if cfg["minor"] else 4
    pad = np.zeros(n)
    for k, iv in enumerate((0, third, 7)):
        f0 = fm * 4 * 2 ** (iv / 12)
        trem = 1 + 0.10 * np.sin(2 * np.pi * (0.11 + 0.07 * k) * t)
        for det in (-0.6, 0.0, 0.6):
            for h, ha in ((1, 1.0), (2, 0.35), (3, 0.12)):
                pad += ha * trem * np.sin(2 * np.pi * (f0 + det) * h * t + k)
    pad *= cfg["pad_amp"] / 9.0
    return (marker + pad) * fade_env(n, XFADE)


def noise_bed(n, cfg, rng):
    x = rng.standard_normal(n)
    b, a = signal.butter(2, cfg["noise_lp"] / (SR / 2))
    y = signal.lfilter(b, a, x)
    y *= cfg["noise_amp"] / (np.std(y) + 1e-9)
    return y


def pulse(n, cfg, t0):
    if cfg["pulse"] == 0.0:
        return np.zeros(n)
    bpm = 78.0
    period = 60.0 / bpm
    t = (np.arange(n) / SR) + t0
    ph = np.mod(t, period)
    env = np.minimum(ph / 0.030, 1.0) * np.exp(-ph / 0.22)
    return cfg["pulse"] * env * np.sin(2 * np.pi * 55.0 * t)


def generate(name, cfg, secs):
    slots = secs // SLOT_SECS
    seq = sequence(cfg["seed"], slots)
    rng = np.random.default_rng(cfg["seed"] + 1000)
    total = secs * SR
    out = np.zeros(total + int(2 * XFADE * SR))
    for k, r in enumerate(seq):
        seg = render_slot(r, cfg, None, k * SLOT_SECS - XFADE)
        start = int(k * SLOT_SECS * SR)
        out[start:start + len(seg)] += seg
    out = out[:total]
    out += noise_bed(total, cfg, rng)
    out += pulse(total, cfg, 0.0)
    peak = np.max(np.abs(out))
    out *= PEAK_TARGET / peak
    # global soft fade in/out (2 s) to avoid any onset click at 0 and at the loop point
    f = int(2.0 * SR)
    ramp = 0.5 - 0.5 * np.cos(np.linspace(0, np.pi, f))
    out[:f] *= ramp
    out[-f:] *= ramp[::-1]
    # stereo: slight decorrelation by a 7 ms delay on the right channel
    d = int(0.007 * SR)
    left = out
    right = np.concatenate([np.zeros(d), out[:-d]])
    st = np.stack([left, right], axis=1).astype("<f4")
    return st, seq


def ffmpeg_version():
    """The ffmpeg build identifier (the token after `ffmpeg version`), recorded in the manifest."""
    out = subprocess.run(["ffmpeg", "-version"], check=True, capture_output=True, text=True).stdout
    match = re.match(r"ffmpeg version (\S+)", out)
    assert match, "unrecognised ffmpeg -version output"
    return match.group(1)


def encode(pcm, path, kbps):
    p = subprocess.Popen(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "f32le", "-ar", str(SR), "-ac", "2", "-i", "-",
         "-fflags", "+bitexact", "-flags:a", "+bitexact", "-c:a", "libvorbis", "-b:a", f"{kbps}k", path],
        stdin=subprocess.PIPE,
    )
    p.communicate(pcm.tobytes())
    assert p.returncode == 0


def main():
    outdir = Path(sys.argv[1])
    secs = 1200
    if "--secs" in sys.argv:
        secs = int(sys.argv[sys.argv.index("--secs") + 1])
    manifest = {
        "sample_rate": SR,
        "slot_secs": SLOT_SECS,
        "marker_base_hz": MARKER_BASE_HZ,
        "ffmpeg_version": ffmpeg_version(),
        "numpy_version": np.__version__,
        "tracks": {},
    }
    for name, cfg in TRACKS.items():
        pcm, seq = generate(name, cfg, secs)
        encode(pcm, str(outdir / f"{name}.ogg"), 130)
        manifest["tracks"][name] = {"duration_secs": secs, "sequence": seq}
        print("generated", name, secs, "s")
    with open(outdir / "manifest.json", "w", newline=chr(10)) as f:
        json.dump(manifest, f, indent=1)
        f.write(chr(10))


if __name__ == "__main__":
    main()

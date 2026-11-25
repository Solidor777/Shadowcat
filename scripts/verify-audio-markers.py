#!/usr/bin/env python3
"""Decode an audio file with ffmpeg and recover the position code: the strongest spectral peak in 100..215 Hz
inside each slot's central window, mapped to the nearest semitone above MARKER_BASE_HZ.

Usage: verify-audio-markers.py FILE MANIFEST.json TRACK_NAME
Prints mismatches and a final line `MARKERS slots=N mismatches=M`.
"""
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

path, manifest_path, name = sys.argv[1:4]
manifest = json.loads(Path(manifest_path).read_text(encoding="utf-8"))
seq = manifest["tracks"][name]["sequence"]
slot = manifest["slot_secs"]
base = manifest["marker_base_hz"]
sr = 48000
raw = subprocess.run(
    ["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", path, "-f", "f32le", "-ac", "1", "-ar", str(sr), "-"],
    check=True, capture_output=True,
).stdout
pcm = np.frombuffer(raw, dtype="<f4")
n_slots = min(len(seq), len(pcm) // (slot * sr))
bad = 0
nfft = 1 << 18
freqs = np.fft.rfftfreq(nfft, 1 / sr)
band = (freqs >= 100) & (freqs <= 215)
win = np.hanning(int(4 * sr))
for k in range(n_slots):
    a = int((k * slot + 3) * sr)
    seg = pcm[a:a + int(4 * sr)]
    if len(seg) < len(win):
        break
    spec = np.abs(np.fft.rfft(seg * win, nfft))
    f = freqs[band][np.argmax(spec[band])]
    r = int(round(12 * np.log2(f / base)))
    if r != seq[k]:
        bad += 1
        print(f"slot {k}: expected {seq[k]} got {r} ({f:.1f} Hz)")
print(f"MARKERS slots={n_slots} mismatches={bad}")

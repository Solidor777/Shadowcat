# Performance

Shadowcat's stage redraws a WebGL canvas continuously, and on a mid-range
phone or a GPU-less host that costs a whole core and a warm battery. The
performance settings (Settings panel → **Performance**) cap that cost: they
are **per-device**, stored in the browser's `localStorage`, never synced to
your account — a phone and a desktop on the same account keep their own
budgets.

## Presets

A preset is a named bundle of every knob below. Picking **Auto** (the default)
lets the client choose from your device's signals; editing any single field
flips the preset to **Custom**, carrying every other field forward at its
current value.

| Setting | Mobile | Balanced | Quality |
|---|---|---|---|
| Frame-rate cap | 30 | 60 | Uncapped |
| Render scale | 0.75 | 1 | 1 |
| Antialiasing | off | on | on |
| Token effects | off | on | on |
| Lighting quality | Static (no sweeps) | Full | Full |
| Visual effects | off | on | on |
| 3D dice | off | on | on |
| Spatial audio | on | on | on |
| Skip redraws when idle | on | on | on |
| Reduce motion | off | off | off |

**Why auto?** Auto resolves to the Mobile budget when the device looks
constrained — a coarse (touch) pointer on a narrow (compact) viewport, four or
fewer CPU cores (`hardwareConcurrency`), or 4 GiB or less of reported device
memory — and to Balanced otherwise. Separately, the operating system's
"reduce motion" accessibility preference is always honoured on top of
whichever preset is resolved, without forcing you onto a custom preset.

## What each knob costs — and what turning it down buys

- **Frame-rate cap** — the ticker's maximum rate. The cap is the single
  biggest lever on CPU/GPU time: 30 fps halves the worst-case frame work of
  60. "Uncapped" maps to the renderer's own no-limit mode.
- **Render scale** — the resolution the scene is rasterized at, as a fraction
  of the device pixel ratio (0.5–1). Pixel work scales with the square of
  this, so 0.75 is roughly 44% fewer pixels to shade; the canvas is upscaled
  back to full size, at some sharpness cost.
- **Antialiasing** — multisampling smooths token and shape edges. It is fixed
  when the renderer initializes, so toggling it rebuilds the stage (a brief
  reload); every other knob applies live.
- **Token effects** — per-token color filters (condition tints, desaturation,
  highlights). Their cost scales with the token count; off drops them all
  except the selection highlight, which is bounded by how many tokens you
  have selected.
- **Lighting quality** — the cosmetic darkness/tint overlay. "Static" keeps
  the overlay but snaps moving lights to their final frame (no per-frame
  interpolation); "Off" skips the overlay entirely. This never changes what
  you can or cannot see — fog of war and vision secrecy are untouched.
- **Visual effects** — sprite VFX playback. Off skips the layer's reconcile
  and one-shot playback.
- **3D dice** — the simulated dice overlay. Off leaves roll results as chat
  cards only, and drops the second WebGL context entirely.
- **Spatial audio** — positional mixing of audio emitters. Off mixes every
  emitter flat at its channel gain.
- **Skip redraws when idle** — the stage only redraws when something actually
  changed (a move, a sweep, an animated token) instead of every frame. On a
  static scene this is the difference between a busy core and an idle one.
- **Reduce motion** — token tweens and light/vision sweeps snap to their end
  state in one step. Pings and emotes keep their timing (they are signals,
  not motion).

## Frame stats

The **Show frame stats** toggle adds a live `fps · ms` readout to the status
bar (at most four updates a second). It is a transient display preference —
never persisted — meant for checking whether a knob change actually moved the
frame rate on this device.

# Walls

A wall is a line segment in a scene. Each wall decides, separately, whether it
stops sight, light, sound and movement. The GM draws walls with the wall tool
and edits one by selecting it with the select tool; the wall editor opens
beside the tool rail.

## Blocking flags

| Control | Effect |
|---|---|
| Blocks sight | Vision rays stop at the wall. |
| Blocks light | Light stops at the wall. |
| Blocks sound | Carried sound is occluded. When this is never set, the wall blocks sound exactly when it blocks sight. |
| Blocks movement | Tokens cannot cross the wall. |

## Modifiers

- **Door.** `closed`, `open` or `locked`. An open door is a hole in the wall.
  Only the GM can lock or unlock a door.
- **One-way.** The wall is solid from one side only, for sight and movement.
  The flip button swaps the solid side. Light and sound stay two-sided.
- **Limited sight.** A sight ray may cross one such wall and stops at the next
  one: see through one, not two.
- **Elevation band.** The wall applies only between the bottom and top
  elevations. Both ends empty means every elevation.

## See-through when near

A wall can open for a viewer, a light or a listener that stands close to it. A
hedge, a bead curtain or a frosted pane works this way: up close you see
through, from far away the wall blocks as usual.

| Control | Meaning |
|---|---|
| See-through when near | Turns the tier on. Off removes it and the wall behaves as before. |
| Open within (cells) | The distance, in grid cells, from the wall at which it opens. A distance of exactly that many cells opens it. Hex and square grids use the same unit. |
| Sight, Light, Sound | The senses the tier applies to. Each is independent: a Sight-only tier leaves light and sound blocked. At least one stays checked. |
| Sound volume through the wall | Shown with Sound. How loud a sound stays once it passes the open wall, from silent to full. Full when never set. |

The distance is measured from the origin of what is sensing: the viewer's
position for sight, the light's position for light, the listener for sound. The
whole wall opens or closes together; it never opens along part of its length.

Movement is never affected: a wall that blocks movement still blocks it while
open for the senses. Players cannot change a tier; it is an ordinary wall
edit, which only the GM can make.

On the canvas a wall with a tier is drawn dashed so the GM can tell it from an
ordinary wall.

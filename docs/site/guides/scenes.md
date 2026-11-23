# Scenes and regions

Scene margin, portals with several destinations, and regions that follow a token.

## Scene margin

A scene can have a margin: extra room around its authored bounds, measured in
grid cells. Set it in the scene panel of the game settings, in **Margin around the
scene (cells)** (0 to 100, whole steps).

- The margin is added on all four sides. Empty and 0 mean no margin.
- Tokens can stand in the margin, and routes, sight and light cross it.
- A positive margin widens the area a player is told they can see.
- The GM sees an outline of the authored bounds while the margin is above 0.
- On hex grids the margin is scaled to the hex size.
- A value outside 0 to 100, or one that is not a finite number, is refused rather
  than clamped.

## Portals with several destinations

A region trigger can send a token to a fixed place (**Teleport**) or ask the player
where to go (**Several destinations**).

In the region editor, add the effect **Several destinations**. It has an optional
prompt and 1 to 8 options. Each option has a label (non-empty, at most 120
characters), a target, and a **reveal scene** switch. With the switch off, the
chooser shows only the label; with it on, it also shows the destination scene's name.

When a token enters or is arrested by the region, the server makes an offer for that
token. The offer lasts 120 seconds, and a newer offer for the same token replaces it.
The offer carries labels only, never coordinates.

- Offers reach the GM and, for a region everyone can see, the token's effective
  owner. A GM-only region offers to the GM only.
- The chooser is a modal titled "Choose a destination". Each option is a large
  touch target. Escape or **Stay here** declines. An option whose scene no longer
  exists shows "Unavailable". Several offers queue, oldest first.
- A **Use portal** button re-issues the offer for a token that is standing in the
  region.
- A choice teleports the token the same way a fixed Teleport does. A choice is refused
  where the region does not allow teleport. A player sees only "portal rejected"; the
  GM sees the reason.
- While the session is [paused](/guides/session-pause), a player cannot accept a
  choice. Declining and opening still work.

## Regions that follow a token

A region can be anchored to a carrier token (**Follows token**). It moves with that
token. Two more settings apply: **Affects the token it follows** and **Turns with the
token**.

If the carrier token is deleted or is not on the scene, the region is **dormant**: it
is off the map and cannot be picked there. Dormant regions are listed in the GM region
controls under **Regions without their token**. Each row has:

- **Edit**;
- **Follow selected token**, enabled only when exactly one valid token is selected on
  the scene, which makes the region follow it;
- **Delete**.

# Tokens

How tokens show their name and resources, how you edit and delete them from the
actors panel, and how conditions and creature senses change their look.

## Bars and nameplates

A token can show a **nameplate** (its name) and up to four **bars** (a current
value over a maximum, such as hit points). The server works out what each player
may see and sends only that, so a player never receives a bar they may not see.

### Authoring

A GM sets nameplates and bars on an actor row in the actors panel, under
**Nameplate and attribute bars**. The create form has the same controls. Each bar has:

- a label (non-empty, at most 64 characters);
- a **current** formula and a **max** formula, evaluated over the actor's `system`
  data;
- a colour (`#rrggbb`);
- a display mode.

A new bar starts as label "Bar", current 0, max 1, colour `#cc2222`, mode hover.
The Add button is disabled at 4 bars.

| Mode label | Who is sent the bar |
|---|---|
| Never | nobody |
| To owner and GM, on hover | the token's effective owner and the GM; shown on hover or selection (the default) |
| To owner and GM, always | the token's effective owner and the GM; always shown |
| To everyone, always | everyone who can read the token |

The nameplate has its own display mode, set the same way.

### What a player sees

- A token contributes only when its scene is visible to the viewer and the token
  itself is visible.
- Formulas are evaluated over the viewer's own redacted copy of the actor. A bar
  whose formula reads a GM-only value is left out for that viewer.
- One bar that fails to evaluate never removes the others, and a value that is not
  a finite number is skipped.
- The nameplate shows the token's name override, else the actor's name, else the
  token's own display name. When the viewer cannot see the name (a secret
  identity), the plate reads "Unknown".

### Per-token overrides

A linked token can replace the actor's nameplate mode and bars for that one token.
In the token controls, the nameplate select has an inherit option, and the bars
select switches between **Inherit** and **Custom**; Custom starts from a copy of the
actor's bars. An override replaces the actor's list as a whole. An instanced token
reads its own embedded actor and has no overrides.

### Turning them off on a device

The **Token nameplates and bars** toggle in the [Performance](/guides/performance)
settings hides every nameplate and bar on that device without changing any data.

## Quick edit and delete

When exactly one token is selected, the actors panel shows a quick-edit block for it:

- **Elevation** control.
- A condition toggle palette.
- A **Bar step** input and, for each bar delivered to you, minus and plus buttons
  around the value, with "/ max".

A step writes the actor value that the bar's current formula names, and only when
that formula is a bare reference to one value. For a linked token the write goes to
the actor; for an instanced token, to the token's embedded actor. A bar whose
current value is computed is disabled ("This value is computed; edit its source
instead."), and so is a bar you may not edit ("You cannot edit this value.").
If someone else changed the value first, the write is refused rather than
overwriting it.

### Deleting a token

A **Delete token** button shows only where you may delete that token. The GM may
delete any token. A token's effective owner (the token's own owner setting, else the
linked actor's owner) may delete that token and nothing else. Owning a token does
not let you manage its embedded documents or edit its permissions.

## Conditions

### Overlays

A condition can be marked **Large overlay** in the conditions panel. The first overlay
condition in a token's condition order draws one large glyph, centred at 60% of the
token's smaller side, over the art and under the badge chips. The condition's badge
chip stays.

A world gets nine default conditions the first time a GM enters it: dead, unconscious,
prone, stunned, poisoned, blinded, invisible, hasted and slowed. Only dead and
unconscious start as overlays. The seed runs once and does not change a registry that
already exists.

## Sensed tokens

A token a viewer sees only through a creature sense (tremorsense and similar) is drawn
with a cyan tint, above the fog layer. The tint is not part of the **Token effects**
performance toggle, so turning that off does not remove it.

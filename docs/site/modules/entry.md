# entry

## Purpose

The pre-world entry experience: login, first-run setup, and world selection.
The shell renders it for any pre-world route. A self-hoster can replace this
package to integrate external auth/identity without touching the in-world UI.

## Contributions

None — `entry` is not a contract-registered module; it exports the `Entry`
component directly, and the shell mounts it for pre-world routes. It is the one
"module" that runs before a world session (and therefore before the module
registry) exists.

## Components

- `Entry.svelte` — the combined login / setup / world-select flow.

## Contracts & seams

- Consumed by the shell's router directly (no `provides`/`requires`).

## Pointers

- Source: `src/modules/entry/`
- API: [`@shadowcat/module-entry`](/api/ts/modules/_shadowcat_module-entry.html)

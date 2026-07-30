# sheet-fallback

## Purpose

The always-registered generic document sheet: registered under the sheet
**fallback** contract at `-Infinity` priority, so any doc_type-specific
provider always wins — but every document, of any type, can still be opened.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-fallback:sheet` | `SHEET_FALLBACK_CONTRACT` | `FallbackSheet` | sheet priority `-Infinity` |

## Components

- `FallbackSheet.svelte` — envelope fields + the opaque `system` tree editor.

## Contracts & seams

- **Provides** the fallback sheet contract (multi). The sheet registry's
  `pickSheet` resolves doc_type-specific `shadowcat.sheet:<doc_type>`
  providers first and lands here only when none exists.
- Replaceable: a game-system module can register its own fallback at higher
  priority.

## Pointers

- Source: `src/modules/sheet-fallback/`
- API: [`@shadowcat/module-sheet-fallback`](/api/ts/modules/_shadowcat_module-sheet-fallback.html)

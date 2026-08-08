// Prints the active TypeDoc documentation-exemption count. An exemption that
// does not announce itself is indistinguishable from a rule that does not apply.
// Cross-platform: node:path/node:fs only.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

/**
 * Reads the enumerated documentation exemptions off a parsed TypeDoc config.
 *
 * @param {{ intentionallyNotDocumented?: string[] }} config - a parsed TypeDoc config object.
 * @returns {{ count: number, names: string[] }} the number of exempted reflection names and
 *   the names themselves, in the order they appear in the config.
 */
export function reportDocExemptions(config) {
  const names = config.intentionallyNotDocumented ?? [];
  return { count: names.length, names };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  // entryPointStrategy: "packages" re-reads options PER PACKAGE from that package's own
  // typedoc.json (via `extends`), discarding root typedoc.json's own keys for anything
  // other than root-only options (e.g. treatValidationWarningsAsErrors). The 8 generated
  // discriminants are exempted in src/types/typedoc.json specifically, because that is the
  // only package whose reflections actually need the exemption; placing it in the shared
  // typedoc.base.json (extended by every other package) makes each of those packages flag
  // all 8 names "unused" for names they never reference.
  const config = JSON.parse(readFileSync(resolve(repo, "src", "types", "typedoc.json"), "utf8"));
  const { count, names } = reportDocExemptions(config);
  console.log(`typedoc: ${count} documentation exemption(s) active`);
  for (const n of names) console.log(`  exempt: ${n}`);
}

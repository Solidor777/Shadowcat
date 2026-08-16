// Property/type/named-arrow doc-coverage lint.
//
// Every rule is an error. There is no advisory tier: a warning is a violation that ships, and it
// is indistinguishable to every later reader from code that was checked and passed.
//
// A SEPARATE ESLint invocation from eslint.docs.config.js, not an extension of it. Both configs
// set the same rule KEYS with different `contexts` lists, and flat config resolves a key to the
// last block that sets it — merged into one config, whichever block came later would silently
// replace the other's context list, dropping that coverage with no error and no output change.
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

// Properties (4): object/class fields, interface members, enum members.
const PROPERTY_CONTEXTS = [
  "PropertyDefinition",
  "TSPropertySignature",
  "TSMethodSignature",
  "TSEnumMember",
];
// Type declarations (3), plus an index signature (1) — declared interface
// contract shape, not a value-bearing property or a param/return-bearing
// signature, so it joins jsdoc/require-jsdoc's and require-description's
// context lists but not require-param/require-returns's.
const TYPE_CONTEXTS = [
  "TSInterfaceDeclaration",
  "TSTypeAliasDeclaration",
  "TSEnumDeclaration",
  "TSIndexSignature",
];
// Named arrow/function expressions (4): deliberately narrow to exported or
// module-level `const`/`let` bindings, never a bare
// ArrowFunctionExpression/FunctionExpression selector — that would also match
// an inline callback argument, which the function-doc gate's own
// `ArrowFunctionExpression: false` rationale (over-firing on inline
// callbacks) already rejects. These selectors cannot match an inline
// callback because an argument position is never a VariableDeclarator.
const ARROW_CONTEXTS = [
  "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
  "ExportNamedDeclaration > VariableDeclaration > VariableDeclarator > FunctionExpression",
  "Program > VariableDeclaration > VariableDeclarator > ArrowFunctionExpression",
  "Program > VariableDeclaration > VariableDeclarator > FunctionExpression",
];
const ALL_CONTEXTS = [...PROPERTY_CONTEXTS, ...TYPE_CONTEXTS, ...ARROW_CONTEXTS];
// TSMethodSignature plus the four arrow/function-expression selectors: the
// only contexts among the twelve that have parameters or a return value to
// document. A plain property, an index signature, or an
// interface/type-alias/enum declaration or member has neither — attaching
// require-param/require-returns there would demand tags that describe
// nothing.
const PARAM_RETURN_CONTEXTS = ["TSMethodSignature", ...ARROW_CONTEXTS];

// Every `require:` entry the function-doc gate covers
// (FunctionDeclaration/ClassDeclaration/MethodDefinition) is explicitly
// `false` here — this config gates only the twelve contexts above, so the two
// configs never both assert requirements about the same construct. Declining
// `ArrowFunctionExpression`/`FunctionExpression` mirrors the function-doc gate's
// own choice to leave them unrequired, rather than duplicating a requirement
// that gate already enforces — this config introduces no requirement on the
// bare selectors either way; its ARROW_CONTEXTS entries reach only the four
// narrow, named-binding paths.
const RULES = {
  "jsdoc/require-jsdoc": ["error", {
    require: {
      FunctionDeclaration: false,
      MethodDefinition: false,
      ClassDeclaration: false,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    contexts: ALL_CONTEXTS,
  }],
  // `contexts` REPLACES the plugin's default list for a rule, not adds to it.
  // Its default (`ArrowFunctionExpression`/`FunctionDeclaration`/
  // `FunctionExpression`/`TSDeclareFunction`) is a function-shaped list that
  // would keep these three rules blind to all twelve contexts above if left
  // implicit — losing that default's function coverage inside THIS config is
  // fine, because eslint.docs.config.js already enforces it at `error`.
  "jsdoc/require-description": ["error", { contexts: ALL_CONTEXTS }],
  "jsdoc/require-param": ["error", { contexts: PARAM_RETURN_CONTEXTS }],
  "jsdoc/require-param-description": ["error", { contexts: PARAM_RETURN_CONTEXTS }],
  "jsdoc/require-returns": ["error", { contexts: PARAM_RETURN_CONTEXTS }],
  // Deliberately NOT extended to any of the twelve contexts: an `@example` on
  // every interface property is noise, not documentation, and would inflate
  // `docs:check-examples`'s compiled-example count for no reader value. Stays
  // on the plugin's function-shaped default list only.
  "jsdoc/require-example": ["error", { exemptNoArguments: false }],
};

// These rules gate on tag PRESENCE only. They cannot detect a vacuous tag, an
// orphaned second block, or a property doc that restates the field name — the
// dominant risk here, since a property's name and its description are so easily
// the same words. A clean run is evidence the tags exist, never that the docs
// are good.

export default [
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    ignores: [
      // Identical to eslint.docs.config.js's ignores — see its own comment for
      // why both test-file conventions and the generated-types exemption must
      // all appear.
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/*.spec.ts", "**/vitest.setup.ts",
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    plugins: { jsdoc, "@typescript-eslint": tseslint.plugin },
    rules: RULES,
  },
  // Mirrors eslint.docs.config.js's hooks.ts exemption: a registered-but-
  // ruleless plugin makes an inline eslint-disable naming its rules dead
  // FROM THIS CONFIG's perspective even though eslint.config.js genuinely
  // needs it. Scoped to the single file that carries such a directive.
  {
    files: ["src/client/core/src/hooks.ts"],
    linterOptions: { reportUnusedDisableDirectives: false },
  },
  {
    files: ["src/client/**/*.svelte", "src/modules/**/*.svelte", "examples/**/*.svelte"],
    ignores: ["**/node_modules/**", "**/dist/**"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
    },
    plugins: { jsdoc },
    rules: RULES,
  },
];

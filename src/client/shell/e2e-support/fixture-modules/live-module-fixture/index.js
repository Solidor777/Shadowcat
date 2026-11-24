// Test-only installed module: ships one stylesheet (`style.css`) and registers nothing else, so
// its presence in a running client is observable as the injected `<link>` and the custom property
// the stylesheet sets. The Playwright loopback server discovers it through `SHADOWCAT_MODULES_DIR`;
// it is never part of the built client (`dist/`) or the release binary, and no world enables it
// unless a spec does.
export default {
  manifest: {
    id: "live-module-fixture",
    version: "1.0.0",
    dependencies: {},
    engines: { shadowcat: "*" },
    style: "style.css",
  },
  register() {},
};

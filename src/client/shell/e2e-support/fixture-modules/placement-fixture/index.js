// Test-only installed module: declares module-placed effects (`placements` in `module.json`).
// The Playwright loopback server discovers it through `SHADOWCAT_MODULES_DIR`; it is never part of
// the built client (`dist/`) or the release binary, and no world enables it unless a spec does.
// The server authorises every placement from the manifest declaration, so this module registers
// nothing on the client.
export default {
  manifest: {
    id: "placement-fixture",
    version: "1.0.0",
    dependencies: {},
    engines: { shadowcat: "*" },
  },
  register() {},
};

import { defineConfig } from "vitest/config";

// e2e run: only the Node<->Rust suite, which spawns the Rust test_server and
// drives the real client over a WebSocket. Generous timeouts cover the server
// build/boot. Single-threaded: each suite owns a server process.
export default defineConfig({
  test: {
    include: ["**/*.e2e.test.ts"],
    testTimeout: 60_000,
    hookTimeout: 240_000,
    fileParallelism: false,
  },
});

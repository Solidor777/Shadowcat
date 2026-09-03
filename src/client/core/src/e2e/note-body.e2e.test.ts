// Node<->Rust end-to-end: `buildNoteDoc`/`parseNoteBody` against the real
// server's `data::engine::note` ingress. Covers the one behavior no unit test
// can: a real wire round-trip through `Room::publish`'s redaction, so a
// private-by-default note is invisible to a player watcher who never
// received `Owner`/`Observer` on it, while the GM's own echo carries the
// server-derived body (html/roll_button/doc_link) — never the client's
// placeholder `body: []`.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireCommand, WireDocument } from "../wire";
import type { RejectReason } from "@shadowcat/types";
import { buildNoteDoc, parseNoteBody, NOTE_DOC_TYPE } from "../note-docs";
import { isKnownSegment } from "../chat-docs";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => {
  server = await startTestServer();
});
afterAll(() => server?.stop());

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () =>
        resolve({ send: (d: string) => sock.send(d), close: () => sock.close() }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Fetches the caller's own account id via `/api/me`, using `cookie`'s session.
 * @param baseUrl The test server's base URL.
 * @param cookie The session cookie to authenticate with.
 * @returns The account id.
 */
async function meId(baseUrl: string, cookie: string): Promise<string> {
  const res = await fetch(`${baseUrl}/api/me`, { headers: { cookie } });
  if (!res.ok) throw new Error(`/api/me failed: ${res.status}`);
  const body = (await res.json()) as { id: string };
  return body.id;
}

/** Fetches a document by id via the HTTP by-id route (READ-gated, redacted).
 * @param baseUrl The test server's base URL.
 * @param cookie The session cookie to authenticate with.
 * @param id The document id.
 * @returns The document, or `null` on a 404.
 */
async function getDoc(baseUrl: string, cookie: string, id: string): Promise<WireDocument | null> {
  const res = await fetch(`${baseUrl}/api/documents/${id}`, { headers: { cookie } });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`get_document failed: ${res.status}`);
  return (await res.json()) as WireDocument;
}

/** Finds a Create op's document by id in a received command.
 * @param cmd The received command.
 * @param id The target document's id.
 * @returns The document, or `undefined` when no Create op in `cmd` targets `id`.
 */
function createdDoc(cmd: WireCommand, id: string): WireDocument | undefined {
  for (const op of cmd.ops) {
    if (op.op === "create" && op.doc.id === id) return op.doc;
  }
  return undefined;
}

test("note bodies are server-derived, private by default, and re-derive on a source edit", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const { world, doc: fixtureDoc } = server.fixture;
  const gmId = await meId(server.baseUrl, gmCookie);

  let rejected: RejectReason | null = null;
  const gm = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: () => {},
      onReject: (_id, reason) => {
        rejected = reason;
      },
    },
  });
  await gm.start();
  await sleep(300);

  let plSawNote = false;
  const plWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === NOTE_DOC_TYPE) plSawNote = true;
        }
      },
    },
  });
  await plWatch.start();
  await sleep(300);

  const noteId = "20000000-0000-0000-0000-000000000001";
  const sourceWithSpans = `# Hi\n\n[[roll:1d6|Luck]] see [[doc:${fixtureDoc}|Doc]]`;
  const doc = buildNoteDoc(world, "Session 1", sourceWithSpans, { owner: gmId, id: noteId });

  let echoed: WireDocument | undefined;
  const gmReader = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        const found = createdDoc(cmd, noteId);
        if (found) echoed = found;
      },
    },
  });
  await gmReader.start();
  await sleep(300);

  gm.send({ type: "intent", intent_id: crypto.randomUUID(), ops: [{ op: "create", doc }] });
  for (let i = 0; i < 50 && !echoed; i++) await sleep(100);

  expect(echoed).toBeDefined();
  const segments = echoed ? parseNoteBody(echoed) : null;
  expect(segments).not.toBeNull();
  expect(segments?.some((s) => isKnownSegment(s) && s.kind === "html")).toBe(true);
  expect(segments?.some((s) => isKnownSegment(s) && s.kind === "roll_button")).toBe(true);
  expect(segments?.some((s) => isKnownSegment(s) && s.kind === "doc_link")).toBe(true);

  // A player has no standing on a private-by-default note: no create ever reaches them.
  await sleep(200);
  expect(plSawNote).toBe(false);

  // Editing /engine/source re-derives body: read the post-update state back via HTTP.
  gm.send({
    type: "intent",
    intent_id: crypto.randomUUID(),
    ops: [
      {
        op: "update",
        doc_id: noteId,
        changes: [{ path: "/engine/source", old: sourceWithSpans, new: "**bold** only" }],
      },
    ],
  } as ClientMsg);
  await sleep(400);
  const afterEdit = await getDoc(server.baseUrl, gmCookie, noteId);
  expect(afterEdit).not.toBeNull();
  const updatedSegments = afterEdit ? parseNoteBody(afterEdit) : null;
  expect(updatedSegments).toEqual([{ kind: "html", sanitized_html: "<p><strong>bold</strong> only</p>\n" }]);

  // A malformed doc-link span in the source is a rejected intent, not a silently-truncated body.
  gm.send({
    type: "intent",
    intent_id: crypto.randomUUID(),
    ops: [
      {
        op: "update",
        doc_id: noteId,
        changes: [{ path: "/engine/source", old: "**bold** only", new: "[[doc:nope|x]]" }],
      },
    ],
  } as ClientMsg);
  for (let i = 0; i < 50 && rejected === null; i++) await sleep(100);
  expect(rejected).toBe("invalid");

  // A child note whose parent is a NON-note (the fixture doc) is rejected.
  rejected = null;
  const childId = "20000000-0000-0000-0000-000000000002";
  const child = buildNoteDoc(world, null, "child", { owner: gmId, id: childId, parentId: fixtureDoc });
  gm.send({ type: "intent", intent_id: crypto.randomUUID(), ops: [{ op: "create", doc: child }] });
  for (let i = 0; i < 50 && rejected === null; i++) await sleep(100);
  expect(rejected).toBe("invalid");

  gm.stop();
  gmReader.stop();
  plWatch.stop();
});

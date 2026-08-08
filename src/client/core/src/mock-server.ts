// An in-process TypeScript implementation of the WS server protocol, used to
// drive the WS client end-to-end in tests without the Rust server (the `web` CI
// job has no Rust toolchain). It assigns seqs, echoes intents as authoritative
// Events, broadcasts to all connections, serves resync from a log, and supports
// a scripted reject rule. NOT production code.
import type { RejectReason } from "@shadowcat/types";
import type { Connect, Transport, TransportHandlers } from "./transport";
import type { ClientMsg, ServerMsg, WireCommand, WireOperation } from "./wire";

/** One simulated connection, as tracked internally (not exported). Unlike a real server
 * connection, `author` is supplied by the caller at `connector()` time rather than derived from
 * an authenticated session — the mock does not authenticate. */
interface Conn {
  /** Locally-assigned connection id (`MockServer.nextId`), scoped to this `MockServer` instance. */
  id: number;
  /** The author id this connection sends commands as; not authenticated, unlike a real login. */
  author: string;
  /** The `WsClient`-supplied callbacks this connection delivers frames to. */
  handlers: TransportHandlers;
  /** Whether this connection is still open; `sendTo` is a no-op once `false`. */
  open: boolean;
}

/** An intent as passed to a `MockServerOptions.rejectRule`. */
export interface IntentContext {
  /** The client-generated intent id (used only for the reject frame if rejected). */
  intentId: string;
  /** The intent's operations. */
  ops: WireOperation[];
  /** The author id of the connection that sent the intent. */
  author: string;
}

/** Configuration for a `MockServer` instance. */
export interface MockServerOptions {
  /** The simulated world id every connection shares; defaults to `"test-world"`. */
  world?: string;
  /** Clock override for `server_time`/`ts`/`expires_at`-style fields; defaults to a fixed `1000`,
   * unlike a real server's wall-clock `now()` — tests wanting elapsed time must supply this. */
  now?: () => number;
  /** Return a reason to reject an intent, or null to accept it. */
  rejectRule?: (ctx: IntentContext) => RejectReason | null;
}

/** An in-process TypeScript stand-in for the server protocol, driving the
 * WS client end-to-end in tests without the Rust server (the `web` CI job has
 * no Rust toolchain). It assigns seqs, echoes intents as authoritative Events,
 * broadcasts to every connection, serves resync from an in-memory log, and
 * supports a scripted reject rule. NOT production code — a test double, not a
 * protocol-conformance reference. */
export class MockServer {
  /** The server's current authoritative sequence number; incremented once per accepted intent. */
  private seq = 0;
  /** The full resync log, in `seq` order, replayed from an arbitrary `fromSeq` by `handleResync`.
   * Unlike a real server, this is an unbounded in-memory list with no persistence or truncation. */
  private log: WireCommand[] = [];
  /** Currently open connections, keyed by `Conn.id`. */
  private conns = new Map<number, Conn>();
  /** Next `Conn.id` to assign; monotonic, never reused even after a connection drops. */
  private nextId = 1;
  /** The simulated world id every connection shares (`MockServerOptions.world`, defaulted). */
  private readonly world: string;
  /** The clock every timestamped frame reads (`MockServerOptions.now`, defaulted). */
  private readonly now: () => number;

  /** Builds a mock server. Each `connector()`/`connect()` call it drives yields
   * an independent simulated connection sharing this instance's world/log/clock.
   * @param opts Configuration for the simulated world.
   * @example
   * ```
   * // test scaffolding — not part of @shadowcat/core's public surface;
   * // import directly from mock-server.ts within the core package
   * import { MockServer } from "./mock-server";
   *
   * const server = new MockServer({ world: "test-world" });
   * ```
   */
  constructor(private readonly opts: MockServerOptions = {}) {
    this.world = opts.world ?? "test-world";
    this.now = opts.now ?? (() => 1000);
  }

  /** A `Connect` for a client authenticating as `author`. Passing the returned
   * function to a `WsClient` (as its `connect` option) sends a `welcome` frame
   * immediately, then dispatches every subsequent message the client sends
   * through this mock server's intent/resync/ping handling.
   * @param author The connecting client's author id, attributed to every
   * command it authors.
   * @returns A `Connect` function suitable for `WsClientOptions.connect`.
   * @example
   * ```
   * // test scaffolding — not part of @shadowcat/core's public surface
   * import { MockServer } from "./mock-server";
   *
   * const server = new MockServer();
   * const connect = server.connector("00000000-0000-0000-0000-000000000001");
   * ```
   */
  connector(author: string): Connect {
    return (handlers) => {
      const conn: Conn = { id: this.nextId++, author, handlers, open: true };
      this.conns.set(conn.id, conn);
      const transport: Transport = {
        send: (data) => this.onClientMessage(conn, data),
        close: () => this.dropConn(conn),
      };
      this.sendTo(conn, {
        type: "welcome",
        world: this.world,
        current_seq: this.seq,
        server_time: this.now(),
        server_version: "0.0.0-test",
        world_default_grants: { by_role: {}, by_user: {} },
        user_role: "player",
        capability_requirements: [],
        contract_declarations: [],
        schema_declarations: [],
      });
      return Promise.resolve(transport);
    };
  }

  /** Server-initiated disconnect (simulates a dropped connection), for every
   * open connection currently authored by `author`.
   * @param author The author id whose connection(s) to drop.
   * @example
   * ```
   * // test scaffolding — not part of @shadowcat/core's public surface
   * import { MockServer } from "./mock-server";
   *
   * const server = new MockServer();
   * server.drop("00000000-0000-0000-0000-000000000001");
   * ```
   */
  drop(author: string): void {
    for (const conn of this.conns.values()) {
      if (conn.author === author) this.dropConn(conn);
    }
  }

  /** The server's current authoritative sequence number.
   * @returns The highest `seq` assigned to any accepted intent so far.
   * @example
   * ```
   * // test scaffolding — not part of @shadowcat/core's public surface
   * import { MockServer } from "./mock-server";
   *
   * const server = new MockServer();
   * server.currentSeq(); // 0
   * ```
   */
  currentSeq(): number {
    return this.seq;
  }

  /** Closes `conn` and fires its `onClose` handler; a no-op if already closed.
   * Not exported — folded into `drop`'s public surface.
   * @param conn The connection to close.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const conn: Conn;
   * this.dropConn(conn);
   * ```
   */
  private dropConn(conn: Conn): void {
    if (!conn.open) return;
    conn.open = false;
    this.conns.delete(conn.id);
    conn.handlers.onClose();
  }

  /** Routes one client-sent frame to its handler (`intent`, `resync_request`,
   * `time_ping`); silently drops unparseable JSON. `hello`/`pong` frames are
   * accepted but produce no response. Not exported — the `Transport.send`
   * implementation `connector` wires up.
   * @param conn The sending connection.
   * @param data The raw JSON message text.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const conn: Conn;
   * declare const data: string;
   * this.onClientMessage(conn, data);
   * ```
   */
  private onClientMessage(conn: Conn, data: string): void {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(data) as ClientMsg;
    } catch {
      return;
    }
    switch (msg.type) {
      case "intent":
        this.handleIntent(conn, msg.intent_id, msg.ops);
        break;
      case "resync_request":
        this.handleResync(conn, msg.from_seq);
        break;
      case "time_ping":
        this.sendTo(conn, {
          type: "time_pong",
          client_t0: msg.client_t0,
          server_t: this.now(),
        });
        break;
      case "hello":
      case "pong":
        break;
    }
  }

  /** Accepts or rejects one client intent. When `opts.rejectRule` returns a
   * reason, sends a `reject` frame naming it back to the sending connection
   * only. Otherwise assigns the next `seq`, appends the resulting `WireCommand`
   * to the resync log, and broadcasts it to every connection (including the
   * sender) as an `event` frame with `intent_id: null` — echoes never carry the
   * originating intent id; a `WsClient` recognizes its own echo by matching
   * `command.author` against its own connection's author instead. Not exported —
   * folded into `onClientMessage`'s public surface.
   * @param conn The sending connection.
   * @param intentId The client-generated intent id (used only for the reject
   * frame, never echoed on acceptance).
   * @param ops The intent's operations.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const conn: Conn;
   * this.handleIntent(conn, "intent-1", []);
   * ```
   */
  private handleIntent(
    conn: Conn,
    intentId: string,
    ops: WireOperation[],
  ): void {
    const reason =
      this.opts.rejectRule?.({ intentId, ops, author: conn.author }) ?? null;
    if (reason) {
      this.sendTo(conn, { type: "reject", intent_id: intentId, reason });
      return;
    }
    this.seq += 1;
    const cmd: WireCommand = {
      seq: this.seq,
      world_id: this.world,
      author: conn.author,
      ts: this.now(),
      ops,
    };
    this.log.push(cmd);
    this.broadcast({ type: "event", command: cmd, intent_id: null });
  }

  /** Replays the resync log from `fromSeq` to `conn`: a `resync_begin` frame,
   * one `event` frame per logged command at or after `fromSeq`, then a
   * `resync_end` frame carrying the server's current `seq`. When no logged
   * command matches (`fromSeq` already caught up), `resync_begin.to_seq` is
   * `fromSeq - 1` — an empty replay window, mirroring an up-to-date resync.
   * Not exported — folded into `onClientMessage`'s public surface.
   * @param conn The requesting connection.
   * @param fromSeq The first seq the client wants replayed.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const conn: Conn;
   * this.handleResync(conn, 1);
   * ```
   */
  private handleResync(conn: Conn, fromSeq: number): void {
    const events = this.log.filter((c) => c.seq >= fromSeq);
    const toSeq = events.length ? events[events.length - 1].seq : fromSeq - 1;
    this.sendTo(conn, {
      type: "resync_begin",
      from_seq: fromSeq,
      to_seq: toSeq,
      source: "log",
    });
    for (const c of events) {
      this.sendTo(conn, { type: "event", command: c, intent_id: null });
    }
    this.sendTo(conn, { type: "resync_end", current_seq: this.seq });
  }

  /** Sends `msg` to every currently open connection, including the one that
   * caused it (e.g. the author of an accepted intent gets their own echo back).
   * Not exported — folded into `handleIntent`'s public surface.
   * @param msg The frame to broadcast.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const command: WireCommand;
   * this.broadcast({ type: "event", command, intent_id: null });
   * ```
   */
  private broadcast(msg: ServerMsg): void {
    for (const conn of this.conns.values()) this.sendTo(conn, msg);
  }

  /** Delivers `msg` to `conn` on a macrotask (`setTimeout(..., 0)`), mirroring a
   * real socket: server responses are never synchronous with the client's send,
   * so optimistic predictions are observable before confirm/reject, and Welcome
   * lands only after the client has assigned its transport. Timers fire FIFO,
   * so delivery order across calls is preserved. A no-op if `conn` has closed by
   * the time the timer fires. Not exported — the sole delivery path every other
   * method routes through.
   * @param conn The destination connection.
   * @param msg The frame to deliver.
   * @example
   * ```
   * // internal helper; not part of the public API
   * declare const conn: Conn;
   * this.sendTo(conn, { type: "resync_end", current_seq: 0 });
   * ```
   */
  private sendTo(conn: Conn, msg: ServerMsg): void {
    const data = JSON.stringify(msg);
    setTimeout(() => {
      if (conn.open) conn.handlers.onMessage(data);
    }, 0);
  }
}

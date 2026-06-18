// An in-process TypeScript implementation of the M5 server protocol, used to
// drive the WS client end-to-end in tests without the Rust server (the `web` CI
// job has no Rust toolchain). It assigns seqs, echoes intents as authoritative
// Events, broadcasts to all connections, serves resync from a log, and supports
// a scripted reject rule. NOT production code.
import type { RejectReason } from "@shadowcat/types";
import type { Connect, Transport, TransportHandlers } from "./transport";
import type { ClientMsg, ServerMsg, WireCommand, WireOperation } from "./wire";

interface Conn {
  id: number;
  author: string;
  handlers: TransportHandlers;
  open: boolean;
}

export interface IntentContext {
  intentId: string;
  ops: WireOperation[];
  author: string;
}

export interface MockServerOptions {
  world?: string;
  now?: () => number;
  /** Return a reason to reject an intent, or null to accept it. */
  rejectRule?: (ctx: IntentContext) => RejectReason | null;
}

export class MockServer {
  private seq = 0;
  private log: WireCommand[] = [];
  private conns = new Map<number, Conn>();
  private nextId = 1;
  private readonly world: string;
  private readonly now: () => number;

  constructor(private readonly opts: MockServerOptions = {}) {
    this.world = opts.world ?? "test-world";
    this.now = opts.now ?? (() => 1000);
  }

  /** A `Connect` for a client authenticating as `author`. */
  connector(author: string): Connect {
    return (handlers) => {
      const conn: Conn = { id: this.nextId++, author, handlers, open: true };
      this.conns.set(conn.id, conn);
      const transport: Transport = {
        send: (data) => this.onClientMessage(conn, data),
        close: () => this.dropConn(conn),
      };
      // Welcome arrives as a later frame (a macrotask), mirroring a real socket
      // where the connection is established before the first message — so the
      // client has assigned its transport by the time Welcome is handled.
      setTimeout(() => {
        if (conn.open) {
          this.sendTo(conn, {
            type: "welcome",
            world: this.world,
            current_seq: this.seq,
            server_time: this.now(),
          });
        }
      }, 0);
      return Promise.resolve(transport);
    };
  }

  /** Server-initiated disconnect (simulates a dropped connection). */
  drop(author: string): void {
    for (const conn of this.conns.values()) {
      if (conn.author === author) this.dropConn(conn);
    }
  }

  currentSeq(): number {
    return this.seq;
  }

  private dropConn(conn: Conn): void {
    if (!conn.open) return;
    conn.open = false;
    this.conns.delete(conn.id);
    conn.handlers.onClose();
  }

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

  private broadcast(msg: ServerMsg): void {
    for (const conn of this.conns.values()) this.sendTo(conn, msg);
  }

  private sendTo(conn: Conn, msg: ServerMsg): void {
    if (conn.open) conn.handlers.onMessage(JSON.stringify(msg));
  }
}

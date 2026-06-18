// WebSocket client over the M5 protocol: maintains an ordered application
// watermark (the client-side sequence guard), recovers gaps via ResyncRequest,
// reconnects with exponential backoff, and tracks the server time offset. It
// emits in-order commands and rejects to its handlers; wiring to the document
// store / optimistic engine is the caller's job.
import type { RejectReason } from "@shadowcat/types";
import { parseServerMsg, type ClientMsg, type WireCommand } from "./wire";
import type { Connect, Transport } from "./transport";

export interface WsClientHandlers {
  /** An in-order, sequence-guarded authoritative command (live or replayed). */
  onCommand(cmd: WireCommand): void;
  /** An intent the server refused. */
  onReject?(intentId: string, reason: RejectReason): void;
  onWelcome?(world: string, currentSeq: number): void;
}

export interface WsClientOptions {
  connect: Connect;
  handlers: WsClientHandlers;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
  backoffBaseMs?: number;
  backoffMaxMs?: number;
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((r) => setTimeout(r, ms));

export class WsClient {
  private transport: Transport | null = null;
  private running = false;
  private reconnectAttempt = 0;
  /** Next seq to apply; the client-side ordering watermark. Persists across
   * reconnects so resync resumes from where application left off. */
  private nextExpected = 1;
  private serverOffsetMs = 0;

  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly backoffBaseMs: number;
  private readonly backoffMaxMs: number;

  constructor(private readonly opts: WsClientOptions) {
    this.now = opts.now ?? Date.now;
    this.sleep = opts.sleep ?? defaultSleep;
    this.backoffBaseMs = opts.backoffBaseMs ?? 250;
    this.backoffMaxMs = opts.backoffMaxMs ?? 10_000;
  }

  /** Open the connection (and keep it open across drops until `stop`). */
  async start(): Promise<void> {
    this.running = true;
    await this.open();
  }

  stop(): void {
    this.running = false;
    this.transport?.close();
    this.transport = null;
  }

  /** Send a client frame (no-op if currently disconnected). */
  send(msg: ClientMsg): void {
    this.transport?.send(JSON.stringify(msg));
  }

  /** The highest authoritative seq applied. */
  get appliedSeq(): number {
    return this.nextExpected - 1;
  }

  /** Estimated server clock. */
  serverNow(): number {
    return this.now() + this.serverOffsetMs;
  }

  private async open(): Promise<void> {
    if (!this.running) return;
    try {
      this.transport = await this.opts.connect({
        onMessage: (d) => this.handleFrame(d),
        onClose: () => this.handleClose(),
      });
      this.reconnectAttempt = 0;
    } catch {
      this.scheduleReconnect();
    }
  }

  private handleClose(): void {
    this.transport = null;
    if (this.running) this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    const attempt = this.reconnectAttempt++;
    const ceiling = Math.min(
      this.backoffMaxMs,
      this.backoffBaseMs * 2 ** attempt,
    );
    const delay = ceiling * (0.5 + Math.random() * 0.5); // full jitter (half..full)
    void this.sleep(delay).then(() => this.open());
  }

  private handleFrame(text: string): void {
    const msg = parseServerMsg(text);
    if (!msg) return;
    switch (msg.type) {
      case "welcome":
        this.serverOffsetMs = msg.server_time - this.now();
        this.opts.handlers.onWelcome?.(msg.world, msg.current_seq);
        // Catch up anything applied-after our watermark (initial sync or a
        // reconnect gap). Idempotent: the server replays from from_seq.
        if (msg.current_seq >= this.nextExpected) {
          this.send({ type: "resync_request", from_seq: this.nextExpected });
        }
        break;
      case "event":
        this.applyEvent(msg.command);
        break;
      case "reject":
        this.opts.handlers.onReject?.(msg.intent_id, msg.reason);
        break;
      case "resync_begin":
        break;
      case "resync_end":
        this.nextExpected = Math.max(this.nextExpected, msg.current_seq + 1);
        break;
      case "time_pong":
        this.serverOffsetMs = msg.server_t - this.now();
        break;
      case "ping":
        this.send({ type: "pong" });
        break;
      case "error":
        break;
    }
  }

  private applyEvent(cmd: WireCommand): void {
    if (cmd.seq < this.nextExpected) return; // duplicate / already applied
    if (cmd.seq > this.nextExpected) {
      // Gap: request the missing prefix; the replay delivers it in order.
      this.send({ type: "resync_request", from_seq: this.nextExpected });
      return;
    }
    this.opts.handlers.onCommand(cmd);
    this.nextExpected = cmd.seq + 1;
  }
}

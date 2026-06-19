import { expect, test } from "vitest";
import { MiddlewareChain } from "./middleware";

test("middleware runs in registration order and can transform ctx", async () => {
  const c = new MiddlewareChain();
  const log: string[] = [];
  c.use("intent-submit", async (ctx: { v: number }, next) => {
    log.push("a-in");
    ctx.v += 1;
    await next();
    log.push("a-out");
  });
  c.use("intent-submit", async (ctx: { v: number }, next) => {
    log.push("b");
    ctx.v *= 10;
    await next();
  });
  const ctx = { v: 1 };
  await c.run("intent-submit", ctx);
  expect(ctx.v).toBe(20); // (1+1)*10
  expect(log).toEqual(["a-in", "b", "a-out"]);
});

test("a middleware that does not call next short-circuits", async () => {
  const c = new MiddlewareChain();
  let reached = false;
  c.use("intent-submit", async () => {
    /* no next() */
  });
  c.use("intent-submit", async (_ctx, next) => {
    reached = true;
    await next();
  });
  await c.run("intent-submit", {});
  expect(reached).toBe(false);
});

test("calling next() twice is rejected", async () => {
  const c = new MiddlewareChain();
  c.use("intent-submit", async (_ctx, next) => {
    await next();
    await next(); // second call must throw
  });
  await expect(c.run("intent-submit", {})).rejects.toThrow(/multiple times/);
});

test("removeModule drops that module's middleware", async () => {
  const c = new MiddlewareChain();
  let ran = false;
  c.use(
    "inbound-event",
    async (_ctx, next) => {
      ran = true;
      await next();
    },
    { module: "m1" },
  );
  c.removeModule("m1");
  await c.run("inbound-event", {});
  expect(ran).toBe(false);
});

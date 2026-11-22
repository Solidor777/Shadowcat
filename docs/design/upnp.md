# Automatic port forwarding (UPnP, NAT-PMP, PCP)

Design of the server's optional router port mapping: how Shadowcat asks a home router to forward
its listening port to the internet, so a self-hosting game master does not edit router settings by
hand. Operator-facing usage belongs in the hosting guide (`docs/site/guides/hosting.md`); the
transport and listener design this builds on is `built-in-https.md`.

**Status.** Designed, not built. No `portmap` module exists. The build is split into a protocol
codec, discovery and clients, runtime wiring, and the admin UI (section 10).

Binding directives: (1) the feature exists to make self-hosting reachable without manual router
work; (2) it is OFF by default, because it opens a port to the internet; (3) it never blocks
startup, and never degrades plain-HTTP or LAN play, whatever the router does; (4) new dependencies
are acceptable when OSI-licensed, but the wire handling stays small and owned (section 4);
(5) cross-platform from day one.

## 1. Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Default | OFF. Turned on by an admin toggle persisted in the database. The operator config key `port_forward` (bool) overrides the toggle and locks it. |
| D2 | What is mapped | ONE TCP mapping: the server's bound port to the same external port when the router grants it. The port serves plain HTTP and TLS (`built-in-https.md` D1), so no second mapping exists. No UDP mapping. |
| D3 | Protocols | NAT-PMP (RFC 6886), PCP (RFC 6887), and UPnP IGD over SSDP. Tried in that order; the first that grants a mapping is the active protocol. |
| D4 | Wire code | Owned: a codec for RFC 6886 / 6887, an SSDP client and a bounded SOAP client. No off-the-shelf IGD client: the available ones hard-code one service version and read response bodies without a cap. |
| D5 | Lifetime | Requested lease 3600 s, renewed at half the granted lease, deleted on shutdown. |
| D6 | Failure | Every failure degrades to a status value and a log line. Nothing in this module can fail startup, panic, or delay accepting connections. |
| D7 | Visibility | Status, including the external address, is admin-only. It is never in a player-visible route. |
| D8 | Module lints | The `portmap` module compiles under `cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::arithmetic_side_effects))`, because it parses bytes from the network. |

## 2. Invariants

1. Packets and documents from the network are untrusted. Every length is checked before use, every
   read is capped, every wait has a deadline, and arithmetic on wire values is checked or saturating.
2. The task that drives mapping is spawned after the listener is bound. Accepting connections never
   waits on it.
3. At most one mapping is held at a time. Every code path that obtains a mapping records it before
   doing anything else that can fail, so shutdown deletes exactly what was created.
4. A router that refuses, times out, or does not exist leaves the server exactly as it was with the
   feature off, plus a status explaining why.
5. A control URL, PCP/NAT-PMP reply, or SSDP reply is accepted only from the address it was asked
   of (section 5.3). A hostile device on the LAN cannot redirect the server's requests to another host.
6. The decision "is mapping enabled" is made in ONE function (`effective_enabled`) that reads the
   config override first and the stored toggle second; the runtime, the admin route and the status
   all call it.

## 3. Enabling

- Stored toggle: key `port_forward_enabled` (`"true"` / `"false"`) in the existing server-wide
  `settings` table, read and written with `get_setting` / `set_setting`. No schema change.
  A missing row means `false`.
- Config override: `port_forward` (bool) in the `Config` layering (CLI flag > `SHADOWCAT_PORT_FORWARD`
  environment > TOML > default, matching every other key). When set it wins over the stored
  toggle in both directions and the admin route reports it as locked.
- `effective_enabled(config, stored) -> bool` is the single decision (invariant 6).
- Toggling at runtime takes effect without a restart: enabling starts the mapping task; disabling
  deletes the mapping and stops the task. A locked toggle rejects the write with 409 and a body
  naming the locking key.
- If the configured `bind` is a loopback address, the feature reports `unreachable_bind` and maps
  nothing: a router cannot forward to a loopback listener.

## 4. Why the wire code is owned

Each protocol handles bytes from an unauthenticated LAN peer, so the surface is small and fixed:
two fixed-size binary protocols and two SOAP actions' worth of XML. A hand-written, bounded codec
is smaller than the audit of a general client and lets the module deny panicking constructs
outright. XML is read by a minimal scanner that extracts the handful of named elements needed
(`serviceType`, `controlURL`, `URLBase`, error codes), never resolves entities or DOCTYPE, and
rejects a document over its cap.

## 5. Protocols

### 5.1 NAT-PMP (RFC 6886) and PCP (RFC 6887)

Both talk UDP to the default gateway on port 5351. The gateway address comes from the platform's
default-route lookup (the `netdev` crate is already a dependency); if none is found the protocol
reports `no_gateway` and discovery moves to SSDP.

- NAT-PMP: version 0. Request for the external address (opcode 0, 2 bytes) and for a TCP mapping
  (opcode 2: version, opcode, 2 reserved, internal port, suggested external port, lifetime: 12
  bytes). A response is 12 bytes (address) or 16 bytes (mapping), opcode + 128, with a result code
  and a seconds-since-start epoch. Lifetime 0 with the internal port deletes the mapping.
- PCP: version 2. A MAP request is the 24-byte common header plus the 36-byte MAP payload (nonce,
  protocol 6, internal port, suggested external port, suggested external address). The nonce is 12
  random bytes per mapping, reused for renewals and for the delete (lifetime 0). A reply
  that is not version 2, has the wrong nonce, or carries a non-zero result code is a failure;
  an `UNSUPP_VERSION` result sends the attempt to NAT-PMP.
- Retransmission follows RFC 6886 section 3.1: initial wait 250 ms, doubling, capped at 5
  attempts here (not the RFC's 9) so a missing gateway resolves in seconds, not minutes.
- Epoch handling: a reply whose epoch advanced slower than elapsed time implies a gateway
  restart, and the mapping is re-requested immediately.
- Replies from any source other than the gateway address are dropped.

### 5.2 UPnP IGD

1. SSDP: send `M-SEARCH` to `239.255.255.250:1900` with `ST` set to the InternetGatewayDevice
   type (versions 1 and 2), `MX: 2`, collecting responses for 3 s. Each response is capped at
   2 KiB; at most 8 distinct responders are considered.
2. Fetch the `LOCATION` device description over plain HTTP with a 64 KiB cap and a 5 s deadline.
3. Walk the description's device tree for a `WANIPConnection` (v1 or v2) service, falling back to
   `WANPPPConnection`. The service type that was found is used verbatim in every SOAP request;
   nothing assumes a version.
4. SOAP over the service's `controlURL` (resolved against `URLBase` or the `LOCATION` origin):
   `GetExternalIPAddress`, `AddPortMapping` (TCP, internal client = the local address the
   gateway can reach, enabled, description `Shadowcat`, lease 3600) and `DeletePortMapping`.
5. Errors: SOAP fault `UPnPError` codes are surfaced. `725` (only permanent leases supported)
   retries once with lease 0, recorded as a permanent mapping that shutdown still deletes.
   `718` (conflict) retries once with an external port chosen by the gateway (suggested port 0
   is not allowed by IGD, so the next unused port in a small window above the preferred one).

### 5.3 Trust of discovered endpoints

- The `LOCATION` URL and every `controlURL` must resolve to the SAME IP address as the SSDP
  responder, over `http`, on a private or link-local address (RFC 1918, unique-local or
  link-local; never loopback or a public address). Anything else is rejected with status `untrusted_endpoint`.
- No redirects are followed. Only the exact request/response shapes above are read.

## 6. Runtime

`PortMapRuntime` (held in `AppState` beside `tls`) owns the task and the status. It is started
after the listener is bound and shut down after the HTTP drain.

States of the task:

1. `Discovering`: try NAT-PMP, then PCP, then IGD (steps in section 5), each within its own
   deadline, all inside one task.
2. `Mapped { protocol, external_addr, external_port, expires }`: renew at half the granted
   lease; on a renewal failure, go to `Discovering` with backoff.
3. `Failed { reason }`: back off 60 s doubling to 15 min, then `Discovering`. `Disabled` and
   `unreachable_bind` are terminal until the toggle or config changes.

Granted external port may differ from the request; the status always reports the port actually
granted and the startup log line `reachable at http://<external>:<port>` is added only for a
confirmed mapping whose external address is public.

Shutdown: `http::serve_with` returns after the drain; `main` then awaits
`PortMapRuntime::shutdown`, which deletes the mapping with a 3 s total deadline and ignores the
result beyond a log line. A process killed hard leaves a mapping that expires with its lease.

Double NAT / carrier-grade NAT: when the reported external address is not publicly routable, the
mapping is kept but the status is `mapped_not_public` with the address, so the admin UI can say
that internet players will not reach it.

## 7. Admin surface

- `GET /api/admin/portmap` (admin only, same extractor as `GET /api/admin/tls`): JSON status:
  `enabled`, `locked_by_config`, `state` (`disabled`, `discovering`, `mapped`, `mapped_not_public`,
  `failed`, `unreachable_bind`), `protocol` (`nat_pmp`, `pcp`, `upnp`), `external_address`,
  `external_port`, `lease_expires_in_secs`, `reason` (machine key) and `detail` (text).
- `PUT /api/admin/portmap` body `{ "enabled": bool }`: stores the toggle and applies it;
  409 when locked.
- No player-visible route carries any of this (invariant 7, D7): the external address is the
  host's home address.

## 8. Security posture

- Enabling opens a TCP port to the whole internet. The admin UI states this next to the toggle,
  and notes that the self-signed certificate warns once per browser and plain HTTP remains
  available (`built-in-https.md`), so the operator should know which origin players will use.
- Default OFF is the safe default. A server that wants it on without UI access sets
  `port_forward = true`.
- Reasons the router protocols are acceptable on a LAN: requests go only to the default
  gateway (NAT-PMP/PCP) or to an address that answered SSDP and matches section 5.3; no
  inbound listener is opened by this module other than the transient UDP socket for a request.
- The external address is operator-sensitive and appears only in admin status and the startup log.

## 9. Tests

- Codec: round trips and malformed-input tables for NAT-PMP and PCP (short, long, wrong
  version, wrong opcode, wrong nonce, non-zero result), each asserted to return an error and
  never panic.
- SOAP and description scanner: fixtures for IGD v1 and v2, nested devices, a PPP service,
  oversized body, DOCTYPE, wrong-host control URL, a fault with 725 and with 718.
- Runtime: a fake gateway on loopback UDP and a fake IGD over loopback TCP drive the state
  machine: map, renew at half lease, epoch regression, refusal, timeout, delete on shutdown,
  and a toggle flip at runtime. A test that the task starts without delaying a bound listener.
- `effective_enabled`: config true/false over stored true/false/missing.
- Admin routes: non-admin 403, locked 409, status shape.

## 10. Build order

1. `portmap` module skeleton, the NAT-PMP / PCP codec and its tests, the module lint block.
2. SSDP discovery, the bounded SOAP client, the IGD flow and its tests.
3. `port_forward` config key, the stored toggle, `effective_enabled`, `PortMapRuntime` in
   `AppState`, the admin routes, shutdown wiring, startup log line.
4. The admin settings section (through the settings-section contribution contract, so the
   settings panel itself is untouched), status display, and the hosting guide section.

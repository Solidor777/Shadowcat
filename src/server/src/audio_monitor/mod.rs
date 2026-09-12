//! `shadowcat audio-monitor`: a localhost-only subcommand serving the peak audio level of
//! watched OS processes (Discord, any voice app) to the ducking module's `OsMonitorSource`
//! over a WebSocket. Never a second executable — a `CliCommand::AudioMonitor` branch of the
//! single `shadowcat` binary.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

//! Automatic router port forwarding: how the server asks a home router to forward its listening
//! port to the internet (NAT-PMP, PCP and UPnP IGD).
//!
//! This module holds the wire codec for the two fixed-size binary protocols ([`codec`]) and the
//! UPnP IGD client: SSDP discovery ([`ssdp`]), the bounded XML/HTTP/SOAP plumbing ([`soap`]) and
//! the discover-describe-map flow ([`igd`]). On top sit the NAT-PMP/PCP UDP exchanges ([`client`]),
//! the protocol ladder that obtains, renews and deletes one mapping ([`mapping`]), the task and
//! enable decision ([`runtime`]) and the admin status ([`status`]). It parses bytes from an unauthenticated LAN peer, so
//! the module denies every construct that can panic: every length is checked before use, every
//! read goes through a bounds-checked reader or `get`, and arithmetic on wire values is checked
//! or saturating.
//!
//! INVARIANT: Nothing in this module can panic on network input. The lint block below makes
//! `unwrap`, `expect`, `panic!`, slice indexing and unchecked arithmetic compile errors outside
//! test builds; adding a construct they reject is a defect, not a lint to silence.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

pub mod client;
pub mod codec;
pub mod igd;
pub mod mapping;
pub mod runtime;
pub mod soap;
pub mod ssdp;
pub mod status;

pub use runtime::PortMapRuntime;

#[cfg(test)]
pub(crate) mod test_support;

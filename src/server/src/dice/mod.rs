//! Server-authoritative dice engine. Pure library: a struct-canonical `RollSpec`
//! is rolled by `roll` (the only randomness step) and scored by `evaluate`
//! (deterministic). Randomness is a stateless noise function, so any roll is
//! reproducible from its seed. INVARIANT: (spec, raws) fully determines the outcome.

pub mod rng;

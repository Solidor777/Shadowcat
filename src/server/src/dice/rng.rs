#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Stateless 64-bit noise (SplitMix64 finalizer). Deterministic: `noise(seed, n)`
/// depends only on its inputs, so any die is reproducible from (seed, index) with no
/// carried state. Source: SplitMix64 [Steele, Lea & Flood 2014]; constants are the
/// published golden-ratio increment + two mixing multipliers. Chosen over a stateful
/// PRNG because a dice engine needs position-based reproducibility for recalculation.
pub fn noise(seed: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(n.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Abstract randomness source: tests seed deterministically; production seeds from
/// entropy at the transport boundary. Trait-object friendly (`&mut dyn RngSource`).
pub trait RngSource {
    /// The next 32 uniformly-distributed bits.
    fn next_u32(&mut self) -> u32;
}

/// Deterministic generator over the noise function: output i = `noise(seed, i)`,
/// advancing an index counter. Reproducible: rebuild with the same seed to replay.
pub struct NoiseRng {
    /// The fixed stream seed.
    seed: u64,
    /// Position in the stream (increments per draw).
    index: u64,
}

impl NoiseRng {
    /// A generator at position 0 of `seed`'s stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::rng::{NoiseRng, RngSource};
    /// let mut a = NoiseRng::from_seed(7);
    /// let mut b = NoiseRng::from_seed(7);
    /// assert_eq!(a.next_u32(), b.next_u32()); // same seed, same stream
    /// ```
    pub fn from_seed(seed: u64) -> Self {
        NoiseRng { seed, index: 0 }
    }

    /// Pure, deterministic function of `(seed, index)` — for schemes that derive a die
    /// directly by explicit index, never through a stateful `next_u32()` sequence.
    /// WARNING: does NOT reproduce the k-th draw of a `next_u32()`/`roll_uniform()`
    /// sequence once any rejection has occurred — `roll_uniform`'s rejection-sampling
    /// loop can consume more than one `next_u32()` call per logical draw, so a rejected
    /// draw shifts every later die's true noise-index away from its ordinal position.
    pub fn at(seed: u64, index: u64) -> u64 {
        noise(seed, index)
    }
}

impl RngSource for NoiseRng {
    fn next_u32(&mut self) -> u32 {
        let v = noise(self.seed, self.index) as u32;
        self.index += 1;
        v
    }
}

/// Unbiased inclusive `[min, max]` draw via rejection sampling (drop the biased tail
/// above the largest multiple of the span). PRECONDITION: `min <= max`. Rejection
/// sampling avoids the modulo bias of `next_u32() % span`.
pub fn roll_uniform(rng: &mut dyn RngSource, min: i32, max: i32) -> i32 {
    debug_assert!(min <= max, "roll_uniform requires min <= max");
    let span = (max as i64 - min as i64 + 1) as u64; // 1..=2^32
    if span == 1 {
        return min;
    }
    if span == 1u64 << 32 {
        // Full u32 range: every possible `x` is a valid draw, no rejection needed
        // (and `span as u32` would truncate to 0, making the modulo below panic).
        return min.wrapping_add(rng.next_u32() as i32);
    }
    let span32 = span as u32;
    // Conservative rejection threshold: drops the entire top residue class rather than
    // computing the tightest exact bound for power-of-two spans. Not a bug — uniformity
    // holds either way, this just rejects marginally more than the minimum necessary.
    let limit = u32::MAX - (u32::MAX % span32);
    loop {
        let x = rng.next_u32();
        if x < limit {
            return min + (x % span32) as i32;
        }
    }
}

#[cfg(test)]
mod tests;

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Stateless 64-bit noise (SplitMix64 finalizer). Deterministic: `noise(seed, n)`
/// depends only on its inputs, so any die is reproducible from (seed, index) with no
/// carried state. Source: SplitMix64 [Steele, Lea & Flood 2014]; constants are the
/// published golden-ratio increment + two mixing multipliers. Chosen over a stateful
/// PRNG because a dice engine needs position-based reproducibility for recalculation.
///
/// # Examples
///
/// ```
/// use shadowcat::dice::rng::noise;
/// // Pure function of its inputs: identical (seed, n) always agrees.
/// assert_eq!(noise(1, 0), noise(1, 0));
/// assert_ne!(noise(1, 0), noise(1, 1));
/// ```
pub fn noise(seed: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(n.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Abstract randomness source: tests seed deterministically; production seeds from
/// entropy at the transport boundary. Trait-object friendly (`&mut dyn RngSource`).
///
/// # Examples
///
/// ```
/// use shadowcat::dice::rng::{NoiseRng, RngSource};
/// let mut rng: Box<dyn RngSource> = Box::new(NoiseRng::from_seed(0));
/// let a = rng.next_u32();
/// let b = rng.next_u32();
/// assert_ne!(a, b); // successive draws advance the stream
/// ```
pub trait RngSource {
    /// The next 32 uniformly-distributed bits.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::rng::{NoiseRng, RngSource};
    /// let mut a = NoiseRng::from_seed(2);
    /// let mut b = NoiseRng::from_seed(2);
    /// assert_eq!(a.next_u32(), b.next_u32());
    /// ```
    fn next_u32(&mut self) -> u32;
}

/// Deterministic generator over the noise function: output i = `noise(seed, i)`,
/// advancing an index counter. Reproducible: rebuild with the same seed to replay.
///
/// # Examples
///
/// ```
/// use shadowcat::dice::rng::{NoiseRng, RngSource};
/// let mut rng = NoiseRng::from_seed(99);
/// let first = rng.next_u32();
/// let second = rng.next_u32();
/// assert_ne!(first, second);
/// // Rebuilding with the same seed replays the identical stream from position 0.
/// let mut replay = NoiseRng::from_seed(99);
/// assert_eq!(replay.next_u32(), first);
/// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::rng::NoiseRng;
    /// // Pure function of (seed, index): identical inputs always agree.
    /// assert_eq!(NoiseRng::at(5, 2), NoiseRng::at(5, 2));
    /// assert_ne!(NoiseRng::at(5, 2), NoiseRng::at(5, 3));
    /// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::dice::rng::{roll_uniform, NoiseRng};
/// let mut rng = NoiseRng::from_seed(13);
/// let face = roll_uniform(&mut rng, 1, 6);
/// assert!((1..=6).contains(&face));
///
/// // A single-value span never draws from the RNG at all.
/// let mut rng = NoiseRng::from_seed(0);
/// assert_eq!(roll_uniform(&mut rng, 4, 4), 4);
/// ```
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

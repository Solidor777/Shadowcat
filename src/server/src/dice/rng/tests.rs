use super::*;

#[test]
fn seeded_is_deterministic() {
    let mut a = NoiseRng::from_seed(42);
    let mut b = NoiseRng::from_seed(42);
    let xs: Vec<i32> = (0..20).map(|_| roll_uniform(&mut a, 1, 6)).collect();
    let ys: Vec<i32> = (0..20).map(|_| roll_uniform(&mut b, 1, 6)).collect();
    assert_eq!(xs, ys);
}

#[test]
fn different_seeds_differ() {
    let mut a = NoiseRng::from_seed(1);
    let mut b = NoiseRng::from_seed(2);
    let xs: Vec<i32> = (0..50).map(|_| roll_uniform(&mut a, 1, 100)).collect();
    let ys: Vec<i32> = (0..50).map(|_| roll_uniform(&mut b, 1, 100)).collect();
    assert_ne!(xs, ys);
}

#[test]
fn roll_uniform_stays_in_range() {
    let mut r = NoiseRng::from_seed(7);
    for _ in 0..1000 {
        let v = roll_uniform(&mut r, 3, 8);
        assert!((3..=8).contains(&v), "out of range: {v}");
    }
}

#[test]
fn roll_uniform_degenerate_range() {
    let mut r = NoiseRng::from_seed(1);
    assert_eq!(roll_uniform(&mut r, 5, 5), 5);
}

#[test]
fn roll_uniform_full_u32_span_does_not_panic() {
    // span = i32::MAX - i32::MIN + 1 == 2^32; truncating to u32 would be 0 and
    // panic on `u32::MAX % span32`. Just assert no panic across many calls —
    // `(min..=max).contains` is trivially true for the full i32 range.
    let mut r = NoiseRng::from_seed(99);
    for _ in 0..500 {
        let _ = roll_uniform(&mut r, i32::MIN, i32::MAX);
    }
}

#[test]
fn roll_uniform_over_face_index_range_stays_in_bounds() {
    // A 3-face die draws an index in 0..=2 via the same roll_uniform used for Numeric.
    let mut r = NoiseRng::from_seed(3);
    for _ in 0..500 {
        let idx = roll_uniform(&mut r, 0, 2);
        assert!((0..=2).contains(&idx));
    }
}

#[test]
fn at_is_positionally_stable() {
    assert_eq!(NoiseRng::at(123, 4), NoiseRng::at(123, 4));
    assert_ne!(NoiseRng::at(123, 4), NoiseRng::at(123, 5));
}

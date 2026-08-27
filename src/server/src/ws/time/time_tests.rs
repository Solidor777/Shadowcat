use super::*;

#[test]
fn calibrate_computes_offset_and_rtt() {
    // client sends at 1000, receives at 1100 (rtt 100); server stamped 2060.
    // midpoint = 1050; offset = 2060 - 1050 = 1010.
    let (offset, rtt) = calibrate(1000, 1100, 2060);
    assert_eq!(rtt, 100);
    assert_eq!(offset, 1010);
}

#[test]
fn now_millis_is_positive_and_monotone_enough() {
    let a = now_millis();
    let b = now_millis();
    assert!(a > 0 && b >= a);
}

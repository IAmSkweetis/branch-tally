//! Bit-exact checks of `pricing` against `reference/token_report.py` (Python 3.14 results,
//! compared as raw f64 bits).

use branch_tally_core::model::{Bucket, Summary};
use branch_tally_core::pricing::{cache_read_rate, cost, price, price_for, sum_costs};

fn b(input: u64, w5: u64, w1h: u64, read: u64, output: u64) -> Bucket {
    Bucket {
        input,
        cache_write_5m: w5,
        cache_write_1h: w1h,
        cache_read: read,
        output,
        thinking: 99,
        calls: 1,
    }
}

fn bits(x: Option<f64>) -> Option<u64> {
    x.map(f64::to_bits)
}

/// Parses Python's `float.hex()` for a positive normal number: `0x1.<13 hex digits>p<exp>`.
fn hex(h: &str) -> Option<u64> {
    let (mant, exp) = h.strip_prefix("0x1.")?.split_once('p')?;
    let frac = u64::from_str_radix(mant, 16).ok()?;
    let exp: i64 = exp.parse().ok()?;
    let biased = u64::try_from(exp + 1023).ok()?;
    Some((biased << 52) | frac)
}

#[track_caller]
fn assert_cost(model: &str, bucket: &Bucket, python_hex: &str) {
    assert_eq!(bits(cost(bucket, model)), hex(python_hex), "{model}");
}

#[test]
fn fable_5_1_uses_its_own_rates_and_cache_read_override() {
    assert_cost(
        "claude-fable-5-1",
        &b(1234, 56789, 4321, 987_654, 3210),
        "0x1.374e22a2c2374p+0",
    );
}

#[test]
fn fable_5_uses_default_cache_read() {
    assert_cost(
        "claude-fable-5",
        &b(1234, 56789, 4321, 987_654, 3210),
        "0x1.f4ef4e0114d2fp+0",
    );
}

#[test]
fn suffixed_ids_match_by_longest_prefix_and_are_not_shadowed() {
    assert_cost(
        "claude-fable-5[1m]",
        &b(777, 88, 9, 123_456_789, 4242),
        "0x1.eeb635a426bb5p+6",
    );
    assert_cost(
        "claude-fable-5-1[1m]",
        &b(777, 88, 9, 123_456_789, 4242),
        "0x1.f15d9513f8db5p+4",
    );
    assert_eq!(price_for("claude-fable-5-1[1m]"), Some((10.0, 50.0)));
    assert_eq!(
        cache_read_rate("claude-fable-5-1[1m]", 10.0).to_bits(),
        0.25_f64.to_bits()
    );
    assert_eq!(
        cache_read_rate("claude-fable-5[1m]", 10.0).to_bits(),
        (10.0_f64 * 0.1).to_bits()
    );
}

#[test]
fn other_families_are_bit_exact() {
    assert_cost(
        "claude-opus-5",
        &b(3, 1_234_567, 7_654_321, 99_999, 12_345),
        "0x1.5278b90214ad3p+6",
    );
    assert_cost(
        "claude-opus-4-8",
        &b(1, 2, 3, 4, 5),
        "0x1.6df3f961804dap-13",
    );
    assert_cost(
        "claude-sonnet-5",
        &b(1_000_003, 333, 17, 5_550_001, 98_765),
        "0x1.064ec0c2d34edp+2",
    );
    assert_cost(
        "claude-sonnet-4-6",
        &b(12_345, 67_890, 13_579, 24_680, 11_223),
        "0x1.190247021d10bp-1",
    );
    assert_cost(
        "claude-haiku-4-5",
        &b(999_999, 111_111, 222_222, 333_333, 444_444),
        "0x1.eb6095d76d39cp+1",
    );
}

#[test]
fn unknown_model_has_no_price() {
    assert_eq!(price_for("claude-nope-1"), None);
    assert_eq!(cost(&b(5, 5, 5, 5, 5), "claude-nope-1"), None);
}

#[test]
fn sum_matches_python_compensated_sum() {
    // Python 3.12+: sum([0.1, 0.2, 0.3]) == 0.6, naive left-to-right gives 0.6000000000000001.
    assert_eq!(sum_costs([0.1, 0.2, 0.3]).to_bits(), 0.6_f64.to_bits());
    assert_eq!(sum_costs([]).to_bits(), 0.0_f64.to_bits());
}

#[test]
fn price_keeps_order_counts_unpriced_and_floors_the_total() {
    let summary = Summary {
        by_model: vec![
            (
                "claude-sonnet-4-6".to_owned(),
                b(12_345, 67_890, 13_579, 24_680, 11_223),
            ),
            ("claude-nope-1".to_owned(), b(5, 5, 5, 5, 5)),
            (
                "claude-fable-5[1m]".to_owned(),
                b(777, 88, 9, 123_456_789, 4242),
            ),
        ],
        ..Summary::default()
    };
    let costs = price(&summary);
    let names: Vec<_> = costs.by_model.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        names,
        ["claude-sonnet-4-6", "claude-nope-1", "claude-fable-5[1m]"]
    );
    assert_eq!(bits(costs.by_model[0].1), hex("0x1.190247021d10bp-1"));
    assert_eq!(costs.by_model[1].1, None);
    assert_eq!(bits(costs.by_model[2].1), hex("0x1.eeb635a426bb5p+6"));
    assert_eq!(costs.unpriced_models, 1);
    // First-seen order, as `as_json` sums.
    let expected = sum_costs([
        f64::from_bits(hex("0x1.190247021d10bp-1").unwrap_or(0)),
        0.0,
        f64::from_bits(hex("0x1.eeb635a426bb5p+6").unwrap_or(0)),
    ]);
    assert_eq!(costs.total.to_bits(), expected.to_bits());
    assert!(costs.is_floor());
}

#[test]
fn empty_summary_costs_nothing() {
    let costs = price(&Summary::default());
    assert_eq!(costs.by_model.len(), 0);
    assert_eq!(costs.total.to_bits(), 0.0_f64.to_bits());
    assert_eq!(costs.unpriced_models, 0);
}

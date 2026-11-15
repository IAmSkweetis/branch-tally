//! The double-count footnote (#67, decision 0005): an overcount, never a floor. It reaches the
//! rendered Markdown and JSON, and is absent (output unchanged) when there is no risk.

use branch_tally_core::model::{
    Costs, DoubleCountRisk, Footnote, LiveMatchFailure, MixedLedger, PriceTableInfo,
    PriceTableSource, Summary,
};
use branch_tally_core::render::{json, markdown};

fn fully_priced() -> Costs {
    Costs {
        price_table: PriceTableInfo {
            as_of: "2026-11-09".to_owned(),
            source: PriceTableSource::Cache,
        },
        ..Costs::default()
    }
}

fn with_risk(risk: Option<DoubleCountRisk>) -> Summary {
    Summary {
        footnote: Footnote {
            double_count: risk,
            ..Footnote::default()
        },
        ..Summary::default()
    }
}

fn mixed() -> MixedLedger {
    MixedLedger {
        unredacted_lines: 3,
        redacted_lines: 5,
        distinct_salts: 2,
    }
}

const HEAD: &str = "**Total may count some calls twice**";

fn md(risk: Option<DoubleCountRisk>) -> String {
    markdown(&with_risk(risk), &fully_priced(), true)
}

#[test]
fn mixed_ledger_is_stated_with_counts_and_the_fix() {
    let out = md(DoubleCountRisk::new(Some(mixed()), None));
    assert!(out.contains(HEAD), "{out}");
    assert!(
        out.contains("mixes 3 unredacted and 5 redacted line(s) under 2 salt(s)"),
        "{out}"
    );
    assert!(out.contains("`branch-tally ledger redact`"), "{out}");
    assert!(!out.contains("could not be matched"), "{out}");
    assert!(!out.contains("at least"), "{out}");
}

#[test]
fn live_failures_each_have_their_own_cause() {
    let cases = [
        (LiveMatchFailure::NoSalt, "there is no local salt"),
        (
            LiveMatchFailure::SaltUnreadable,
            "the local salt could not be read",
        ),
        (
            LiveMatchFailure::FingerprintMismatch,
            "the local salt matches none of the redacted lines",
        ),
        (
            LiveMatchFailure::PartiallyMatched,
            "the local salt matches only some of the redacted lines",
        ),
    ];
    for (f, cause) in cases {
        let out = md(DoubleCountRisk::new(None, Some(f)));
        assert!(out.contains(HEAD), "{out}");
        assert!(
            out.contains("live calls could not be matched to redacted ledger lines"),
            "{out}"
        );
        assert!(out.contains(cause), "{cause}: {out}");
        assert!(!out.contains("ledger redact"), "{out}");
        assert!(!out.contains("at least"), "{out}");
    }
}

#[test]
fn both_causes_are_stated_together() {
    let out = md(DoubleCountRisk::new(
        Some(mixed()),
        Some(LiveMatchFailure::NoSalt),
    ));
    assert_eq!(out.matches(HEAD).count(), 1, "{out}");
    assert!(out.contains("mixes 3 unredacted"), "{out}");
    assert!(out.contains("there is no local salt"), "{out}");
}

#[test]
fn none_leaves_the_output_untouched() {
    let out = md(None);
    assert!(!out.contains("twice"), "{out}");
    assert_eq!(out, markdown(&Summary::default(), &fully_priced(), true));
    let j = json(&with_risk(None), &fully_priced(), true);
    assert!(!j.contains("doubleCount"), "{j}");
}

#[test]
fn json_carries_the_risk_and_stays_valid() {
    let s = with_risk(DoubleCountRisk::new(
        Some(mixed()),
        Some(LiveMatchFailure::FingerprintMismatch),
    ));
    let j = json(&s, &fully_priced(), true);
    let v: serde_json::Value = serde_json::from_str(&j).expect("valid json");
    let d = &v["doubleCount"];
    assert_eq!(d["mixedLedger"]["unredactedLines"], 3, "{j}");
    assert_eq!(d["mixedLedger"]["redactedLines"], 5, "{j}");
    assert_eq!(d["mixedLedger"]["distinctSalts"], 2, "{j}");
    assert_eq!(d["liveMatch"], "fingerprint-mismatch", "{j}");
    let only = json(
        &with_risk(DoubleCountRisk::new(None, Some(LiveMatchFailure::NoSalt))),
        &fully_priced(),
        true,
    );
    let v: serde_json::Value = serde_json::from_str(&only).expect("valid json");
    assert!(v["doubleCount"].get("mixedLedger").is_none(), "{only}");
    assert_eq!(v["doubleCount"]["liveMatch"], "no-salt", "{only}");
}

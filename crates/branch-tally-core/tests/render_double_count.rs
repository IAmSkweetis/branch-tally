//! The double-count footnote (#67, decision 0005): an overcount, never a floor. It reaches the
//! rendered Markdown and JSON, and is absent (output unchanged) when there is no risk.

use branch_tally_core::model::{BranchRow, ProjectReport};
use branch_tally_core::model::{
    Costs, DoubleCountRisk, Footnote, LiveMatchFailure, MixedLedger, PriceTableInfo,
    PriceTableSource, Summary,
};
use branch_tally_core::render::{json, markdown, project_json, project_markdown, project_svg};

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
        out.contains("the ledger holds 3 unredacted and 5 redacted line(s) under 2 salt(s)"),
        "{out}"
    );
    assert!(out.contains("`branch-tally ledger redact`"), "{out}");
    assert!(!out.contains("could not be matched"), "{out}");
    assert!(!out.contains("at least"), "{out}");
}

#[test]
fn redact_advice_is_qualified_only_when_lines_sit_under_more_than_one_salt() {
    const CAVEAT: &str = "can't reconcile lines already hashed under a different salt";
    let two = md(DoubleCountRisk::new(Some(mixed()), None));
    assert!(two.contains(CAVEAT), "{two}");
    let one = md(DoubleCountRisk::new(
        Some(MixedLedger {
            distinct_salts: 1,
            ..mixed()
        }),
        None,
    ));
    assert!(one.contains("`branch-tally ledger redact`"), "{one}");
    assert!(!one.contains(CAVEAT), "{one}");
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
    assert!(out.contains("the ledger holds 3 unredacted"), "{out}");
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

#[test]
fn salts_only_mix_gives_no_redact_advice() {
    let m = MixedLedger {
        unredacted_lines: 0,
        redacted_lines: 4,
        distinct_salts: 2,
    };
    let out = md(DoubleCountRisk::new(Some(m), None));
    assert!(out.contains(HEAD), "{out}");
    assert!(
        out.contains("the ledger holds 4 redacted line(s) hashed under 2 different salts"),
        "{out}"
    );
    assert!(out.contains("two machines"), "{out}");
    assert!(!out.contains("ledger redact"), "{out}");
    assert!(out.contains("This is an overcount, not a floor."), "{out}");
}

fn project(risk: Option<DoubleCountRisk>) -> ProjectReport {
    ProjectReport {
        rows: vec![BranchRow::default()],
        double_count: risk,
        ..ProjectReport::default()
    }
}

#[test]
fn project_markdown_uses_the_same_statement_and_none_is_unchanged() {
    let risk = DoubleCountRisk::new(Some(mixed()), Some(LiveMatchFailure::NoSalt));
    let out = project_markdown(&project(risk));
    assert_eq!(out.matches(HEAD).count(), 1, "{out}");
    assert!(out.contains("the ledger holds 3 unredacted"), "{out}");
    assert!(out.contains("`branch-tally ledger redact`"), "{out}");
    let plain = project_markdown(&project(None));
    assert!(!plain.contains("twice"), "{plain}");
    assert_eq!(
        plain,
        project_markdown(&ProjectReport {
            double_count: None,
            ..project(None)
        })
    );
}

#[test]
fn project_json_carries_the_risk_and_none_is_unchanged() {
    let risk = DoubleCountRisk::new(Some(mixed()), Some(LiveMatchFailure::PartiallyMatched));
    let j = project_json(&project(risk));
    let v: serde_json::Value = serde_json::from_str(&j).expect("valid json");
    assert_eq!(v["doubleCount"]["mixedLedger"]["redactedLines"], 5, "{j}");
    assert_eq!(v["doubleCount"]["liveMatch"], "partially-matched", "{j}");
    assert!(!project_json(&project(None)).contains("doubleCount"));
}

#[test]
fn project_svg_footer_notes_the_overcount_only_when_set() {
    let risk = DoubleCountRisk::new(None, Some(LiveMatchFailure::NoSalt));
    let svg = project_svg(&project(risk));
    assert!(svg.contains("may count some calls twice"), "{svg}");
    assert!(!project_svg(&project(None)).contains("twice"));
}

const NO_BOUND: &str = "the \"at least\" figures here are not a guaranteed lower bound";

#[test]
fn a_floor_and_a_double_count_together_claim_no_bound() {
    let risk = DoubleCountRisk::new(Some(mixed()), None);
    let mut summary = with_risk(risk);
    summary.footnote.totals_overflowed = true;
    let both = markdown(&summary, &fully_priced(), true);
    assert!(both.contains(NO_BOUND), "{both}");
    assert!(
        !both.contains("This is an overcount, not a floor."),
        "{both}"
    );
    // The double count alone is still an overcount.
    let alone = md(risk);
    assert!(
        alone.contains("This is an overcount, not a floor."),
        "{alone}"
    );
    assert!(!alone.contains(NO_BOUND), "{alone}");
    // The project report, floored by an unpriced model.
    let mut p = project(risk);
    p.totals.unpriced_models = vec!["unknown-model".to_owned()];
    let proj = project_markdown(&p);
    assert!(proj.contains("at least"), "{proj}");
    assert!(proj.contains(NO_BOUND), "{proj}");
    assert!(
        !proj.contains("This is an overcount, not a floor."),
        "{proj}"
    );
}

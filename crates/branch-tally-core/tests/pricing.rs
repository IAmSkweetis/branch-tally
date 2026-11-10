//! Bit-exact checks of `pricing` against `reference/token_report.py` (Python 3.14 results,
//! compared as raw f64 bits).

use branch_tally_core::model::{Bucket, PriceTableSource, Summary};
use branch_tally_core::pricing::{PriceTable, cost as cost_with, price as price_with, sum_costs};

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

fn cost(bucket: &Bucket, model: &str) -> Option<f64> {
    cost_with(bucket, model, PriceTable::bundled())
}

fn price(summary: &Summary) -> branch_tally_core::model::Costs {
    price_with(summary, PriceTable::bundled())
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
    let table = PriceTable::bundled();
    let long = table.lookup("claude-fable-5-1[1m]").unwrap();
    assert_eq!((long.input, long.output), (10.0, 50.0));
    assert_eq!(long.cache_read.map(f64::to_bits), Some(0.25_f64.to_bits()));
    let short = table.lookup("claude-fable-5[1m]").unwrap();
    assert_eq!(short.cache_read, None);
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
    assert!(PriceTable::bundled().lookup("claude-nope-1").is_none());
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

// ---- the price table as data (#5) ----------------------------------------------------------

use branch_tally_core::pricing::{
    FeedError, PriceFeed, PriceTableError, ResolveError, TextSource, fetch_table, from_litellm,
    resolve,
};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

const MINIMAL: &str = "as_of = \"2026-01-31\"\n[models.\"claude-x-1\"]\ninput = 3\noutput = 15\n";

#[test]
fn bundled_table_parses_and_has_a_date() {
    let t = PriceTable::bundled();
    assert_eq!(t.as_of(), "2026-11-09");
    assert_eq!(t.source(), PriceTableSource::Bundled);
    assert_eq!(t.models().len(), 23);
}

#[test]
fn price_records_the_tables_date_and_source() {
    let table = PriceTable::parse(MINIMAL, PriceTableSource::Override).unwrap();
    let costs = price_with(&Summary::default(), &table);
    assert_eq!(costs.price_table.as_of, "2026-01-31");
    assert_eq!(costs.price_table.source, PriceTableSource::Override);
}

#[test]
fn minimal_table_gets_default_multipliers_and_prices_through_them() {
    let table = PriceTable::parse(MINIMAL, PriceTableSource::Cache).unwrap();
    // 1M of each: 3 + 3*1.25 + 3*2 + 0.3 + 15
    let one = b(1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000);
    let c = cost_with(&one, "claude-x-1[1m]", &table).unwrap();
    assert!((c - 28.05).abs() < 1e-9, "{c}");
}

#[test]
fn a_table_can_change_the_multipliers_and_a_model_can_override_them() {
    let text = "as_of = \"2026-01-31\"\n[cache_write]\nfive_minute = 2.0\n[cache_read]\ndefault_factor = 0.5\n\
                [models.\"a\"]\ninput = 1\noutput = 1\n\
                [models.\"b\"]\ninput = 1\noutput = 1\ncache_write_5m = 3.0\ncache_read = 0.25\n";
    let t = PriceTable::parse(text, PriceTableSource::Override).unwrap();
    let w5 = b(0, 1_000_000, 0, 0, 0);
    let read = b(0, 0, 0, 1_000_000, 0);
    assert_eq!(cost_with(&w5, "a", &t), Some(2.0));
    assert_eq!(cost_with(&read, "a", &t), Some(0.5));
    assert_eq!(cost_with(&w5, "b", &t), Some(3.0));
    assert_eq!(cost_with(&read, "b", &t), Some(0.25));
}

#[test]
fn malformed_tables_are_typed_errors() {
    let parse = |s: &str| PriceTable::parse(s, PriceTableSource::Override);
    assert!(matches!(
        parse("not toml ["),
        Err(PriceTableError::Invalid(_))
    ));
    assert_eq!(parse("[models]\n"), Err(PriceTableError::MissingAsOf));
    for bad in [
        "2026-13-01",
        "2026-02-30",
        "26-01-01",
        "2026-1-1",
        "today",
        "2026-01-é",
        "é026-01-01",
        "",
    ] {
        let s = format!("as_of = \"{bad}\"\n");
        assert_eq!(parse(&s), Err(PriceTableError::InvalidAsOf), "{bad}");
    }
    assert!(parse("as_of = \"2024-02-29\"\n").is_ok());
    // Unknown key (typo) and negative rate are errors, never defaults.
    assert!(matches!(
        parse("as_of = \"2026-01-01\"\nbogus = 1\n"),
        Err(PriceTableError::Invalid(_))
    ));
    let neg = "as_of = \"2026-01-01\"\n[models.\"m\"]\ninput = -1\noutput = 1\n";
    assert!(matches!(parse(neg), Err(PriceTableError::InvalidRate(_))));
    let missing = "as_of = \"2026-01-01\"\n[models.\"m\"]\ninput = 1\n";
    assert!(matches!(parse(missing), Err(PriceTableError::Invalid(_))));
}

#[test]
fn error_messages_do_not_quote_the_text() {
    let err =
        PriceTable::parse("as_of = \"SECRET-TOKEN\" ] [", PriceTableSource::Cache).unwrap_err();
    assert!(!err.to_string().contains("SECRET-TOKEN"), "{err}");
}

#[test]
fn to_toml_round_trips_and_is_deterministic() {
    let bundled = PriceTable::bundled();
    let text = bundled.to_toml();
    assert_eq!(text, bundled.to_toml());
    let back = PriceTable::parse(&text, PriceTableSource::Bundled).unwrap();
    assert_eq!(&back, bundled);
}

const LITELLM: &str = r#"{
  "sample_spec": {"litellm_provider": "docs", "mode": "chat"},
  "claude-opus-5": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 5e-6, "output_cost_per_token": 2.5e-5,
    "cache_read_input_token_cost": 5e-7, "cache_creation_input_token_cost": 6.25e-6,
    "cache_creation_input_token_cost_above_1hr": 1e-5},
  "claude-fable-5-1": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 1e-5, "output_cost_per_token": 5e-5,
    "cache_read_input_token_cost": 2.5e-7, "cache_creation_input_token_cost": 1.25e-5},
  "claude-odd-1": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 3e-6, "output_cost_per_token": 1.5e-5,
    "cache_creation_input_token_cost": 3.75e-6,
    "cache_creation_input_token_cost_above_1hr": 9e-6},
  "claude-sonnet-5": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 2e-6, "output_cost_per_token": 1e-5,
    "cache_read_input_token_cost": 2e-7},
  "anthropic/claude-opus-5": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 9e-6, "output_cost_per_token": 9e-5},
  "anthropic.claude-opus-5-v1:0": {"litellm_provider": "bedrock", "mode": "chat",
    "input_cost_per_token": 9e-6, "output_cost_per_token": 9e-5},
  "claude-legacy-complete": {"litellm_provider": "anthropic", "mode": "completion",
    "input_cost_per_token": 1e-6, "output_cost_per_token": 1e-6},
  "claude-nocost-1": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 1e-6},
  "gpt-4": {"litellm_provider": "openai", "mode": "chat",
    "input_cost_per_token": 1e-5, "output_cost_per_token": 3e-5}
}"#;

#[test]
fn litellm_keeps_only_unprefixed_anthropic_claude_chat_models() {
    let imp = from_litellm(LITELLM, "2026-11-09").unwrap();
    let ids: Vec<_> = imp.table.models().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "claude-fable-5-1",
            "claude-odd-1",
            "claude-opus-5",
            "claude-sonnet-5"
        ]
    );
    assert_eq!(imp.skipped, ["claude-nocost-1"]);
    assert_eq!(imp.table.as_of(), "2026-11-09");
    assert_eq!(imp.table.source(), PriceTableSource::Cache);
}

#[test]
fn litellm_converts_per_token_to_per_mtok_without_float_noise() {
    let imp = from_litellm(LITELLM, "2026-11-09").unwrap();
    let opus = imp.table.lookup("claude-opus-5").unwrap();
    assert_eq!(opus.input.to_bits(), 5.0_f64.to_bits());
    assert_eq!(opus.output.to_bits(), 25.0_f64.to_bits());
    let odd = imp.table.lookup("claude-odd-1").unwrap();
    assert_eq!(odd.input.to_bits(), 3.0_f64.to_bits());
}

#[test]
fn litellm_derives_overrides_only_where_they_differ_from_the_globals() {
    let imp = from_litellm(LITELLM, "2026-11-09").unwrap();
    let t = &imp.table;
    // 0.1x input and 1.25x / 2x creation: no overrides.
    let opus = t.lookup("claude-opus-5").unwrap();
    assert_eq!(
        (opus.cache_read, opus.cache_write_5m, opus.cache_write_1h),
        (None, None, None)
    );
    let sonnet = t.lookup("claude-sonnet-5").unwrap();
    assert_eq!(sonnet.cache_read, None);
    // Fable 5.1 reads at 0.25, not 1.0.
    assert_eq!(t.lookup("claude-fable-5-1").unwrap().cache_read, Some(0.25));
    // The odd model disagrees on the 1h multiplier (3x): kept per model, not silently globalised.
    let odd = t.lookup("claude-odd-1").unwrap();
    assert_eq!(odd.cache_write_5m, None);
    assert_eq!(odd.cache_write_1h, Some(3.0));
}

#[test]
fn litellm_rejects_non_objects_and_bad_dates() {
    assert_eq!(
        from_litellm("[1]", "2026-11-09").unwrap_err(),
        PriceTableError::BadFeed
    );
    assert_eq!(
        from_litellm("{nope", "2026-11-09").unwrap_err(),
        PriceTableError::BadFeed
    );
    assert_eq!(
        from_litellm("{}", "nov 9").unwrap_err(),
        PriceTableError::InvalidAsOf
    );
}

#[test]
fn litellm_output_survives_a_cache_file_round_trip() {
    let imp = from_litellm(LITELLM, "2026-11-09").unwrap();
    let back = PriceTable::parse(&imp.table.to_toml(), PriceTableSource::Cache).unwrap();
    assert_eq!(back, imp.table);
}

struct FakeFs {
    files: HashMap<PathBuf, Result<String, io::ErrorKind>>,
}

impl FakeFs {
    fn new(files: Vec<(&str, Result<&str, io::ErrorKind>)>) -> Self {
        Self {
            files: files
                .into_iter()
                .map(|(p, r)| (PathBuf::from(p), r.map(str::to_owned)))
                .collect(),
        }
    }
}

impl TextSource for FakeFs {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match self.files.get(path) {
            None => Ok(None),
            Some(Ok(s)) => Ok(Some(s.clone())),
            Some(Err(kind)) => Err(io::Error::from(*kind)),
        }
    }
}

const OVR: &str = ".tally/prices.toml";
const CACHE: &str = "cache/prices.toml";

fn run(fs: &FakeFs) -> Result<PriceTable, ResolveError> {
    resolve(fs, Path::new(OVR), Path::new(CACHE)).map(|r| r.table)
}

#[test]
fn resolver_prefers_override_then_cache_then_bundled() {
    let ovr = "as_of = \"2026-03-01\"\n";
    let cache = "as_of = \"2026-02-01\"\n";
    let both = FakeFs::new(vec![(OVR, Ok(ovr)), (CACHE, Ok(cache))]);
    let t = run(&both).unwrap();
    assert_eq!(
        (t.as_of(), t.source()),
        ("2026-03-01", PriceTableSource::Override)
    );
    // The whole table wins: nothing is merged in from the cache or bundled layers.
    assert_eq!(t.models().len(), 0);

    let only_cache = FakeFs::new(vec![(CACHE, Ok(cache))]);
    let t = run(&only_cache).unwrap();
    assert_eq!(
        (t.as_of(), t.source()),
        ("2026-02-01", PriceTableSource::Cache)
    );

    let none = FakeFs::new(vec![]);
    let t = run(&none).unwrap();
    assert_eq!(t.source(), PriceTableSource::Bundled);
    assert_eq!(&t, PriceTable::bundled());
}

#[test]
fn malformed_override_is_an_error_naming_the_file_not_a_fall_through() {
    let fs = FakeFs::new(vec![
        (OVR, Ok("as_of = \"SECRET\"")),
        (CACHE, Ok("as_of = \"2026-02-01\"\n")),
    ]);
    let err = run(&fs).unwrap_err();
    assert!(matches!(&err, ResolveError::Malformed { path, .. } if path == Path::new(OVR)));
    let msg = err.to_string();
    assert!(msg.contains(OVR), "{msg}");
    assert!(!msg.contains("SECRET"), "{msg}");
}

#[test]
fn malformed_cache_is_an_error_and_unreadable_files_are_named() {
    let fs = FakeFs::new(vec![(CACHE, Ok("garbage ["))]);
    assert!(matches!(
        run(&fs).unwrap_err(),
        ResolveError::Malformed { path, .. } if path == Path::new(CACHE)
    ));
    let fs = FakeFs::new(vec![(OVR, Err(io::ErrorKind::PermissionDenied))]);
    assert!(matches!(
        run(&fs).unwrap_err(),
        ResolveError::Unreadable { path, kind }
            if path == Path::new(OVR) && kind == io::ErrorKind::PermissionDenied
    ));
}

struct FakeFeed(Result<String, FeedError>);

impl PriceFeed for FakeFeed {
    fn fetch(&self) -> Result<String, FeedError> {
        self.0.clone()
    }
}

#[test]
fn fetch_table_converts_the_feed_and_surfaces_failures() {
    let ok = fetch_table(&FakeFeed(Ok(LITELLM.to_owned())), "2026-11-09").unwrap();
    assert_eq!(ok.table.models().len(), 4);
    let down = FakeFeed(Err(FeedError::Unavailable("offline".into())));
    assert!(fetch_table(&down, "2026-11-09").is_err());
    let junk = FakeFeed(Ok("<html>".into()));
    assert!(fetch_table(&junk, "2026-11-09").is_err());
}

#[test]
fn fs_text_reads_files_and_reports_missing_as_none() {
    use branch_tally_core::pricing::FsText;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("p.toml");
    assert_eq!(FsText.read(&p).unwrap(), None);
    std::fs::write(&p, "x").unwrap();
    assert_eq!(FsText.read(&p).unwrap().as_deref(), Some("x"));
}

// ---- subagent row costs ---------------------------------------------------------------------

mod subagent_rows {
    use super::{b, cost, price};
    use branch_tally_core::model::{Bucket, SubagentRow, SubagentSummary, Summary, Workspace};
    use branch_tally_core::pricing::sum_costs;
    use std::collections::BTreeSet;

    fn row(name: &str, by_model: Vec<(&str, Bucket)>) -> SubagentRow {
        let by_model: Vec<(String, Bucket)> = by_model
            .into_iter()
            .map(|(m, k)| (m.to_owned(), k))
            .collect();
        let mut r = SubagentRow {
            workspace: Workspace::Main,
            subagent: name.to_owned(),
            model: String::new(),
            calls: 0,
            tokens: 0,
            agent_ids: BTreeSet::new(),
            by_model,
        };
        let t = r.total();
        r.calls = t.calls;
        r.tokens = t.tokens();
        r
    }

    fn summary(rows: Vec<SubagentRow>, by_model: Vec<(&str, Bucket)>) -> Summary {
        Summary {
            by_model: by_model
                .into_iter()
                .map(|(m, k)| (m.to_owned(), k))
                .collect(),
            subagents: SubagentSummary {
                rows,
                ..SubagentSummary::default()
            },
            ..Summary::default()
        }
    }

    #[test]
    fn two_priced_models_sum_their_own_costs() {
        let (s, f) = (b(1000, 200, 30, 4000, 500), b(10, 20, 30, 40, 50));
        let sum = summary(
            vec![row(
                "a",
                vec![("claude-sonnet-4-6", s), ("claude-fable-5", f)],
            )],
            vec![("claude-sonnet-4-6", s), ("claude-fable-5", f)],
        );
        let costs = price(&sum);
        let want = sum_costs([
            cost(&s, "claude-sonnet-4-6").unwrap_or(0.0),
            cost(&f, "claude-fable-5").unwrap_or(0.0),
        ]);
        let r = costs.subagent_row(0).copied().unwrap_or_default();
        assert_eq!(r.known.to_bits(), want.to_bits());
        assert!(want > 0.0);
        assert_eq!((r.priced_models, r.unpriced_models), (2, 0));
        assert!(!r.is_floor() && !r.is_unpriced());
    }

    #[test]
    fn an_unpriced_model_makes_the_row_a_floor_and_all_unpriced_is_unpriced() {
        let (s, x) = (b(1000, 0, 0, 0, 500), b(5, 5, 5, 5, 5));
        let sum = summary(
            vec![
                row(
                    "floor",
                    vec![("claude-sonnet-4-6", s), ("claude-nope-1", x)],
                ),
                row("none", vec![("claude-nope-1", x)]),
            ],
            vec![("claude-sonnet-4-6", s), ("claude-nope-1", x)],
        );
        let costs = price(&sum);
        let f = costs.subagent_row(0).copied().unwrap_or_default();
        assert!(f.is_floor() && !f.is_unpriced());
        assert_eq!((f.priced_models, f.unpriced_models), (1, 1));
        assert_eq!(
            f.known.to_bits(),
            cost(&s, "claude-sonnet-4-6").unwrap_or(0.0).to_bits()
        );
        let n = costs.subagent_row(1).copied().unwrap_or_default();
        assert!(n.is_unpriced() && n.is_floor());
        assert_eq!(n.known.to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn rows_sum_to_the_section_total_which_is_within_the_overall_total() {
        let (s, f, x) = (
            b(1000, 200, 30, 4000, 500),
            b(10, 20, 30, 40, 50),
            b(5, 5, 5, 5, 5),
        );
        let sum = summary(
            vec![
                row("a", vec![("claude-sonnet-4-6", s), ("claude-nope-1", x)]),
                row("b", vec![("claude-fable-5", f)]),
            ],
            vec![
                ("claude-sonnet-4-6", b(2000, 400, 60, 8000, 1000)),
                ("claude-fable-5", f),
                ("claude-nope-1", x),
            ],
        );
        let costs = price(&sum);
        assert_eq!(costs.subagent_rows.len(), 2);
        let t = costs.subagent_total;
        assert_eq!(
            t.known.to_bits(),
            sum_costs(costs.subagent_rows.iter().map(|r| r.known)).to_bits()
        );
        assert_eq!(
            t.priced_models,
            costs
                .subagent_rows
                .iter()
                .map(|r| r.priced_models)
                .sum::<u64>()
        );
        assert_eq!(t.unpriced_models, 1);
        assert!(t.is_floor());
        assert!(t.known > 0.0 && t.known <= costs.total);
    }

    #[test]
    fn no_rows_means_no_row_costs() {
        let costs = price(&Summary::default());
        assert_eq!(costs.subagent_rows.len(), 0);
        assert!(costs.subagent_total.is_unpriced());
    }
}

//! Every Suite tool in the roster has exactly one verdict row in the study,
//! and the study has no rows for tools that are not in the roster.
use std::collections::BTreeSet;
use std::fs;

fn roster() -> BTreeSet<String> {
    fs::read_to_string("research/tools.txt")
        .expect("research/tools.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn study_rows() -> Vec<String> {
    fs::read_to_string("docs/suite-study.md")
        .expect("docs/suite-study.md")
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once("` |").map(|(name, _)| name.to_string()))
        .collect()
}

#[test]
fn every_tool_has_exactly_one_verdict() {
    let rows = study_rows();
    for tool in roster() {
        let n = rows.iter().filter(|r| **r == tool).count();
        assert_eq!(n, 1, "{tool} has {n} verdict rows in docs/suite-study.md");
    }
}

#[test]
fn no_verdict_without_a_tool() {
    let roster = roster();
    for row in study_rows() {
        assert!(roster.contains(&row), "verdict row for unknown tool {row}");
    }
}

#[test]
fn roster_is_the_full_suite() {
    assert_eq!(roster().len(), 79, "Suite roster changed; re-run the study");
}

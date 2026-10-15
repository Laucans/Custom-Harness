//! What a task may cost, from what tasks of its kind have cost.
//!
//! Pure: the rows come from the ledger through a port, the arithmetic is
//! here. Two readings of the same rows:
//!
//! - **what one task cost**: its rows summed — but a resumed session reports
//!   a cumulative cost (Claude Code since v2.1.277), so two rows of one
//!   session are the same money twice. The total of a task is the sum, over
//!   its (run, stage) pairs, of the largest cost each pair reported.
//! - **what tasks of a kind cost**: the per-task totals of every task
//!   tagged with the same unit and profile, read as a distribution.
//!
//! A budget is the third quartile of that distribution with half again on
//! top, once three tasks have been measured; before that, a constant per
//! unit, generous, because a budget hit mid-task wastes every dollar before
//! it (what the quota gate learnt on #64).

use std::collections::BTreeMap;

/// One ledger row, as the budget reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Charge {
    /// The task.
    pub task: String,
    /// The run.
    pub run: String,
    /// The stage.
    pub stage: String,
    /// The cost the row reported, when it observed one.
    pub cost_usd: Option<f64>,
}

/// What tasks of one kind cost: the distribution of their totals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Typical {
    /// How many tasks were measured.
    pub count: usize,
    /// The median total.
    pub median: f64,
    /// The third quartile.
    pub p75: f64,
}

/// How many measured tasks it takes before the history, rather than the
/// constant, sets the budget.
pub const ENOUGH: usize = 3;

/// What a budget adds on top of the third quartile: room for the task that
/// is a little bigger than its kind.
pub const HEADROOM: f64 = 1.5;

/// The budget of a unit nobody has measured yet, in dollars.
#[must_use]
pub fn fallback(unit: &str) -> f64 {
    match unit {
        "concept" => 3.0,
        "persisted-query" => 4.0,
        "composition" => 6.0,
        "capability" | "data-capability" | "tooling" => 8.0,
        "aggregate" => 15.0,
        "infrastructure" => 20.0,
        // A Micro-UI, and a unit nobody named.
        _ => 10.0,
    }
}

/// What each task cost: per task, the sum over its (run, stage) pairs of
/// the largest cost that pair reported.
#[must_use]
pub fn totals(charges: &[Charge]) -> BTreeMap<String, f64> {
    let mut largest: BTreeMap<(String, String, String), f64> = BTreeMap::new();
    for charge in charges {
        let Some(cost) = charge.cost_usd else {
            continue;
        };
        let key = (
            charge.task.clone(),
            charge.run.clone(),
            charge.stage.clone(),
        );
        let kept = largest.entry(key).or_insert(0.0);
        if cost > *kept {
            *kept = cost;
        }
    }
    let mut totals: BTreeMap<String, f64> = BTreeMap::new();
    for ((task, _, _), cost) in largest {
        *totals.entry(task).or_insert(0.0) += cost;
    }
    totals
}

/// The distribution of these totals; `None` when there is none.
#[must_use]
pub fn typical(totals: &[f64]) -> Option<Typical> {
    if totals.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = totals.iter().copied().filter(|t| t.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(Typical {
        count: sorted.len(),
        median: quantile(&sorted, 1, 2),
        p75: quantile(&sorted, 3, 4),
    })
}

/// The nearest-rank quantile `num/den` of a sorted, non-empty slice: the
/// value at rank `ceil(num * n / den)`, counted from one. Integer
/// arithmetic, so no count a ledger holds is rounded.
fn quantile(sorted: &[f64], num: usize, den: usize) -> f64 {
    let n = sorted.len();
    let rank = (num.saturating_mul(n).saturating_add(den - 1)) / den;
    let index = rank.clamp(1, n) - 1;
    sorted.get(index).copied().unwrap_or(0.0)
}

/// The budget of a task of `unit`, given what its kind has cost.
#[must_use]
pub fn budget_for(unit: &str, typical: Option<&Typical>) -> f64 {
    match typical {
        Some(found) if found.count >= ENOUGH => found.p75 * HEADROOM,
        _ => fallback(unit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charge(task: &str, run: &str, stage: &str, cost: Option<f64>) -> Charge {
        Charge {
            task: task.to_string(),
            run: run.to_string(),
            stage: stage.to_string(),
            cost_usd: cost,
        }
    }

    #[test]
    fn a_task_s_total_sums_its_stages_and_counts_a_resumed_session_once() {
        let rows = [
            charge("61", "r1", "code", Some(1.0)),
            // The same session, resumed: cumulative, so 2.5 includes the 1.0.
            charge("61", "r1", "code", Some(2.5)),
            charge("61", "r1", "create-test", Some(0.5)),
            charge("62", "r2", "code", Some(4.0)),
            charge("62", "r2", "code", None),
        ];
        let totals = totals(&rows);
        assert!((totals["61"] - 3.0).abs() < 1e-9, "{totals:?}");
        assert!((totals["62"] - 4.0).abs() < 1e-9, "{totals:?}");
    }

    #[test]
    fn a_second_run_on_the_same_stage_is_more_money() {
        let rows = [
            charge("61", "r1", "code", Some(2.0)),
            charge("61", "r2", "code", Some(3.0)),
        ];
        assert!((totals(&rows)["61"] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn the_distribution_reads_median_and_third_quartile() {
        let found = typical(&[1.0, 2.0, 3.0, 4.0]).expect("four tasks");
        assert_eq!(found.count, 4);
        assert!((found.median - 2.0).abs() < 1e-9);
        assert!((found.p75 - 3.0).abs() < 1e-9);
        let one = typical(&[7.0]).expect("one task");
        assert!((one.p75 - 7.0).abs() < 1e-9);
        assert!(typical(&[]).is_none());
        assert!(
            typical(&[f64::NAN]).is_none(),
            "nothing finite, nothing typical"
        );
    }

    #[test]
    fn the_budget_follows_the_history_once_there_is_enough_of_it() {
        let few = Typical {
            count: 2,
            median: 30.0,
            p75: 40.0,
        };
        assert!((budget_for("capability", Some(&few)) - fallback("capability")).abs() < 1e-9);
        let enough = Typical {
            count: 3,
            median: 3.0,
            p75: 4.0,
        };
        assert!((budget_for("capability", Some(&enough)) - 6.0).abs() < 1e-9);
        assert!((budget_for("capability", None) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn every_unit_has_a_fallback_and_the_write_side_is_the_dearest() {
        assert!(fallback("aggregate") > fallback("capability"));
        assert!(fallback("infrastructure") > fallback("aggregate"));
        assert!(fallback("concept") < fallback("capability"));
        assert!(fallback("something-new") > 0.0);
    }
}

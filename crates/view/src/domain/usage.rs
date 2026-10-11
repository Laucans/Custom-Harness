//! What the view's own `claude -p` calls cost, summed up for the
//! dashboard: never a run's ledger, which the *Spending* screen already
//! shows beside this.

use serde::Serialize;

use crate::ports::Spend;

/// How many of the newest calls the summary carries in full.
const RECENT: usize = 20;

/// One asker's share.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ByWho {
    /// `creation`, `kit`, `mf`, `notes`, `units`.
    pub who: String,
    /// Summed; `0.0` when none of its calls carried a cost.
    pub cost_usd: f64,
    /// How many calls.
    pub calls: u64,
}

/// What the dashboard shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Every call's cost, summed; a call with no cost adds nothing.
    pub total_usd: f64,
    /// How many calls were kept.
    pub calls: u64,
    /// How many carried no cost — the API answered without one, or the
    /// call never reached a result.
    pub uncosted: u64,
    /// By who, highest spender first, ties broken by name.
    pub by_who: Vec<ByWho>,
    /// The newest calls, in full.
    pub recent: Vec<Spend>,
}

/// Every call kept, summed by who and totalled.
#[must_use]
pub fn summarize(spends: &[Spend]) -> Summary {
    let mut by_who: Vec<ByWho> = Vec::new();
    let mut total = 0.0;
    let mut uncosted = 0;
    for spend in spends {
        match spend.cost_usd {
            Some(cost) => total += cost,
            None => uncosted += 1,
        }
        match by_who.iter_mut().find(|w| w.who == spend.who) {
            Some(w) => {
                w.cost_usd += spend.cost_usd.unwrap_or(0.0);
                w.calls += 1;
            }
            None => by_who.push(ByWho {
                who: spend.who.clone(),
                cost_usd: spend.cost_usd.unwrap_or(0.0),
                calls: 1,
            }),
        }
    }
    by_who.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.who.cmp(&b.who))
    });
    Summary {
        total_usd: total,
        calls: u64::try_from(spends.len()).unwrap_or(u64::MAX),
        uncosted,
        by_who,
        recent: spends.iter().take(RECENT).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn spend(who: &str, cost: Option<f64>) -> Spend {
        Spend {
            at: "2026-10-12T00:00:00Z".to_string(),
            who: who.to_string(),
            model: "m".to_string(),
            mode: "write".to_string(),
            cost_usd: cost,
            duration_ms: Some(10),
            input_tokens: Some(1),
            output_tokens: Some(1),
        }
    }

    #[test]
    fn an_empty_list_summarizes_to_nothing() {
        assert_eq!(summarize(&[]), Summary::default());
    }

    #[test]
    fn calls_are_summed_by_who_the_highest_spender_first() {
        let spends = vec![
            spend("kit", Some(0.1)),
            spend("creation", Some(1.0)),
            spend("creation", Some(0.5)),
            spend("mf", None),
        ];
        let s = summarize(&spends);
        assert!((s.total_usd - 1.6).abs() < 1e-9);
        assert_eq!(s.calls, 4);
        assert_eq!(s.uncosted, 1);
        assert_eq!(
            s.by_who,
            vec![
                ByWho {
                    who: "creation".to_string(),
                    cost_usd: 1.5,
                    calls: 2
                },
                ByWho {
                    who: "kit".to_string(),
                    cost_usd: 0.1,
                    calls: 1
                },
                ByWho {
                    who: "mf".to_string(),
                    cost_usd: 0.0,
                    calls: 1
                },
            ]
        );
    }

    #[test]
    fn ties_in_cost_are_broken_by_name() {
        let spends = vec![spend("mf", Some(1.0)), spend("kit", Some(1.0))];
        let summary = summarize(&spends);
        let who: Vec<&str> = summary.by_who.iter().map(|w| w.who.as_str()).collect();
        assert_eq!(who, ["kit", "mf"]);
    }

    #[test]
    fn only_the_newest_calls_are_carried_in_full() {
        let spends: Vec<Spend> = (0..RECENT + 5)
            .map(|i| spend(&i.to_string(), Some(1.0)))
            .collect();
        let s = summarize(&spends);
        assert_eq!(s.recent.len(), RECENT);
        assert_eq!(usize::try_from(s.calls).unwrap_or(0), RECENT + 5);
    }

    proptest! {
        #[test]
        fn the_total_never_exceeds_the_sum_of_every_cost(
            costs in proptest::collection::vec(proptest::option::of(0.0..100.0f64), 0..30),
        ) {
            let spends: Vec<Spend> = costs.iter().enumerate().map(|(i, c)| spend(&format!("w{}", i % 3), *c)).collect();
            let s = summarize(&spends);
            let expected: f64 = costs.iter().filter_map(|c| *c).sum();
            prop_assert!((s.total_usd - expected).abs() < 1e-6);
            prop_assert_eq!(usize::try_from(s.uncosted).unwrap_or(0), costs.iter().filter(|c| c.is_none()).count());
            let by_who_total: f64 = s.by_who.iter().map(|w| w.cost_usd).sum();
            prop_assert!((by_who_total - expected).abs() < 1e-6);
        }
    }
}

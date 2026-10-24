//! Which of the role's databases a sweep drops.
//!
//! Pure: the listing, the keep list and the time come in as values. A
//! database is stale when every one of these holds:
//!
//! - its name carries its creation time (`names::created_at`) — which
//!   `postgres`, the templates and a product's own database never do;
//! - it is no template (`datistemplate`), whatever its name;
//! - neither `DATABASE_URL` nor `DATABASE_URL_READONLY` names it;
//! - nobody is connected to it;
//! - it was created `older_than` seconds ago or more. A creation time ahead
//!   of the clock (a skewed clock) reads as just created.
//!
//! The adapter's drop is plain, never `WITH (FORCE)`: a test that connects
//! between the listing and the drop keeps its database.

use harness_core::ports::shell::database::OwnedDatabase;

use super::names::created_at;

/// The names of the stale databases among `databases`, in their order.
#[must_use]
pub fn stale(
    databases: &[OwnedDatabase],
    keep: &[String],
    now: u64,
    older_than: u64,
) -> Vec<String> {
    databases
        .iter()
        .filter(|db| !db.template && db.connections == 0 && !keep.contains(&db.name))
        .filter(|db| created_at(&db.name).is_some_and(|at| now.saturating_sub(at) >= older_than))
        .map(|db| db.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_729_770_000;
    const OLDER_THAN: u64 = 1800;
    /// Nothing to drop.
    const KEPT: [String; 0] = [];

    fn idle(name: &str) -> OwnedDatabase {
        OwnedDatabase {
            name: name.to_string(),
            connections: 0,
            template: false,
        }
    }

    fn minted(at: u64) -> String {
        format!("scratch_{at}_4242_0_move_token")
    }

    #[test]
    fn an_idle_dated_database_at_exactly_the_threshold_is_dropped() {
        let at = NOW - OLDER_THAN;
        assert_eq!(
            stale(&[idle(&minted(at))], &[], NOW, OLDER_THAN),
            [minted(at)]
        );
    }

    #[test]
    fn one_second_younger_is_kept() {
        let at = NOW - OLDER_THAN + 1;
        assert_eq!(stale(&[idle(&minted(at))], &[], NOW, OLDER_THAN), KEPT);
    }

    #[test]
    fn a_future_creation_time_is_kept() {
        assert_eq!(
            stale(&[idle(&minted(NOW + 60))], &[], NOW, OLDER_THAN),
            KEPT
        );
    }

    #[test]
    fn a_connected_database_is_never_dropped() {
        let mut held = idle(&minted(NOW - 86_400));
        held.connections = 1;
        assert_eq!(stale(&[held], &[], NOW, OLDER_THAN), KEPT);
    }

    #[test]
    fn templates_and_the_keep_list_win_over_the_grammar() {
        let named = minted(NOW - 86_400);
        let mut template = idle(&named);
        template.template = true;
        assert_eq!(stale(&[template], &[], NOW, OLDER_THAN), KEPT);
        assert_eq!(
            stale(
                &[idle(&named)],
                std::slice::from_ref(&named),
                NOW,
                OLDER_THAN
            ),
            KEPT
        );
        assert_eq!(
            stale(
                &[idle(&named)],
                &["forgedesmondes".to_string()],
                NOW,
                OLDER_THAN
            ),
            [named]
        );
    }

    #[test]
    fn an_empty_keep_list_still_keeps_every_name_without_a_creation_time() {
        // DATABASE_URL_READONLY unset, or a URL that names no database: the
        // grammar alone still keeps the product's and the server's own.
        let listing: Vec<OwnedDatabase> = [
            "forgedesmondes",
            "dnd_helper",
            "app",
            "postgres",
            "move_token_test_moved_4242",
        ]
        .into_iter()
        .map(idle)
        .collect();
        assert_eq!(stale(&listing, &[], u64::MAX, OLDER_THAN), KEPT);
    }

    #[test]
    fn the_stale_come_out_in_the_listing_s_order_among_the_kept() {
        let old = NOW - 86_400;
        let listing = [
            idle(&format!("rules_runtime_{old}_7")),
            idle("forgedesmondes"),
            idle(&minted(NOW)),
            idle(&format!("schema_dump_{old}_8")),
        ];
        assert_eq!(
            stale(&listing, &[], NOW, OLDER_THAN),
            [
                format!("rules_runtime_{old}_7"),
                format!("schema_dump_{old}_8")
            ]
        );
    }
}

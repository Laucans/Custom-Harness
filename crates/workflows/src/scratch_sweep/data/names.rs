//! The names a scratch database carries its creation time in.
//!
//! Three makers, one grammar each:
//!
//! - `scratch_<epoch>_<pid>_<seq>_<label>` — the product's
//!   `platform-scratch-db` guard, one per test (`label` in `[a-z0-9_]`);
//! - `rules_runtime_<epoch>_<pid>` — `scripts/rules-runtime.sh`;
//! - `schema_dump_<epoch>_<pid>` — `scripts/dump-schema.sh`.
//!
//! `<epoch>` is ten digits of Unix seconds (`date +%s`), the others digits.
//! Anything else — a product's own database, `postgres`, a template, an
//! older test's `<name>_<pid>`, a name with an uppercase letter or a quote —
//! has no creation time, and the sweep never drops it.

/// The digits of a Unix time in seconds, from 2001 to 2286.
const EPOCH_DIGITS: usize = 10;

/// The prefix of the guard's names.
const SCRATCH: &str = "scratch_";

/// The prefixes of the installed scripts' names.
const SCRIPTS: [&str; 2] = ["rules_runtime_", "schema_dump_"];

/// When the database `name` was created, in Unix seconds, read off its
/// name; `None` when its name carries no creation time.
#[must_use]
pub fn created_at(name: &str) -> Option<u64> {
    if let Some(rest) = name.strip_prefix(SCRATCH) {
        let mut parts = rest.splitn(4, '_');
        let at = epoch(parts.next()?)?;
        digits(parts.next()?)?;
        digits(parts.next()?)?;
        let label = parts.next()?;
        let readable = !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        return readable.then_some(at);
    }
    let rest = SCRIPTS
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))?;
    let (at, pid) = rest.split_once('_')?;
    digits(pid)?;
    epoch(at)
}

/// `Some` when `text` is one digit or more, and nothing else.
fn digits(text: &str) -> Option<()> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then_some(())
}

/// The Unix time `text` spells in exactly ten digits.
fn epoch(text: &str) -> Option<u64> {
    digits(text)?;
    if text.len() != EPOCH_DIGITS {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_minted_name_reads_its_epoch() {
        assert_eq!(
            created_at("scratch_1729760000_4242_0_move_token"),
            Some(1_729_760_000)
        );
        assert_eq!(
            created_at("scratch_1729760000_4242_17_move_token_moved_2"),
            Some(1_729_760_000),
            "a label may hold digits and underscores"
        );
    }

    #[test]
    fn the_scripts_names_read_their_epoch() {
        assert_eq!(
            created_at("rules_runtime_1729760000_123"),
            Some(1_729_760_000)
        );
        assert_eq!(created_at("schema_dump_1729760001_9"), Some(1_729_760_001));
    }

    #[test]
    fn a_name_short_of_its_grammar_has_no_creation_time() {
        for name in [
            "scratch_1729760000_4242_0_",
            "scratch_1729760000_4242_0",
            "scratch_1729760000_4242",
            "scratch_1729760000",
            "scratch_",
            "scratch_1729760000__0_x",
            "scratch_1729760000_42_x_label",
            "scratch_172976000_4242_0_x",
            "scratch_17297600000_4242_0_x",
            "scratch_17297600x0_4242_0_x",
            "scratch_1729760000_4242_0_Move",
            "scratch_1729760000_4242_0_a-b",
            "rules_runtime_1729760000_",
            "rules_runtime_1729760000",
            "rules_runtime_1729760000_12x",
            "rules_runtime_1729760000_12_x",
            "rules_runtime_172976000_12",
            "schema_dump__12",
        ] {
            assert_eq!(created_at(name), None, "{name}");
        }
    }

    #[test]
    fn product_system_and_older_test_names_are_never_scratch() {
        for name in [
            "forgedesmondes",
            "dnd_helper",
            "postgres",
            "template0",
            "template1",
            "app",
            "x_1",
            "move_token_test_moved_4242",
            "get_settings_read_4242",
            "media_read_battlemap_4242",
            "campaign_2024",
        ] {
            assert_eq!(created_at(name), None, "{name}");
        }
    }

    #[test]
    fn the_installed_makers_mint_the_grammar_the_sweep_reads() {
        use harness_architecture::assets::content_of;
        let guard = content_of("crates/platform/scratch-db/src/lib.rs").expect("installed");
        assert!(
            guard.contains(r#"format!("scratch_{epoch}_{pid}_{seq}_{label}")"#),
            "the guard's names are the sweep's"
        );
        assert!(guard.contains("since.as_secs()"), "its epoch is in seconds");
        let runtime = content_of("scripts/rules-runtime.sh").expect("installed");
        assert!(runtime.contains(r#"name="rules_runtime_$(date +%s)_$$""#));
        let dump = content_of("scripts/dump-schema.sh").expect("installed");
        assert!(dump.contains(r#"scratch="schema_dump_$(date +%s)_$$""#));
    }

    proptest! {
        #[test]
        fn a_minted_name_round_trips_its_epoch(
            at in 1_000_000_000u64..=9_999_999_999,
            pid in 1u32..,
            seq in 0u32..,
            label in "[a-z0-9_]{1,24}",
        ) {
            prop_assert_eq!(created_at(&format!("scratch_{at}_{pid}_{seq}_{label}")), Some(at));
            prop_assert_eq!(created_at(&format!("rules_runtime_{at}_{pid}")), Some(at));
            prop_assert_eq!(created_at(&format!("schema_dump_{at}_{pid}")), Some(at));
        }

        #[test]
        fn a_name_with_a_character_outside_the_grammar_has_no_creation_time(
            at in 1_000_000_000u64..=9_999_999_999,
            before in "[a-z0-9_]{0,8}",
            odd in "[A-Z\"' .;-]",
            after in "[a-z0-9_]{0,8}",
        ) {
            let label = format!("{before}{odd}{after}");
            prop_assert_eq!(created_at(&format!("scratch_{at}_1_2_{label}")), None);
            prop_assert_eq!(created_at(&format!("rules_runtime_{at}_{label}")), None);
        }

        #[test]
        fn no_name_without_a_known_prefix_has_a_creation_time(name in "[a-z][a-z0-9_]{0,40}") {
            prop_assume!(!name.starts_with(SCRATCH));
            prop_assume!(SCRIPTS.iter().all(|prefix| !name.starts_with(prefix)));
            prop_assert_eq!(created_at(&name), None);
        }
    }
}

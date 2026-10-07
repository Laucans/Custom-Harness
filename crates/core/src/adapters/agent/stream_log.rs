//! What one `--output-format stream-json` line says, in one readable line.
//!
//! Pure, and separate from the call for that reason: the carrier spawns the
//! process and appends what this returns, and every rendering decision tests
//! against a string.
//!
//! # Why this exists
//!
//! `--output-format json` returns one object when the turn is over. A turn
//! that runs for fifty minutes is therefore a black box while it runs: no way
//! to tell a session reasoning about its ninth file from one hung on a network
//! call. `stream-json` emits one JSON object per event, as they happen, and
//! this module turns each into a line a human can follow with `tail -f`.
//!
//! # What it keeps
//!
//! Tool calls and their most telling argument, tool results, the assistant's
//! prose, and the final cost. Not the token-level deltas (the carrier does not
//! ask for `--include-partial-messages`): the question this log answers is
//! "what is it doing, and where did it stall", and a thousand text fragments
//! per minute answer it worse than one line per action.

use serde_json::Value;

/// How much of a free-text value is kept on a line.
///
/// Enough to recognise a command or a sentence, short enough that the log
/// stays scannable in a terminal.
const WIDTH: usize = 160;

/// The input field that best says what a tool call is doing, in the order we
/// prefer them.
///
/// `command` first because `Bash` is where a session spends its time, and the
/// command is the whole story. A tool whose input has none of these is shown
/// by its own name alone — better than a line of JSON braces.
const TELLING: [&str; 8] = [
    "command",
    "file_path",
    "pattern",
    "path",
    "url",
    "query",
    "prompt",
    "description",
];

/// Text with its whitespace collapsed, cut to `width` characters.
///
/// Collapsed because a tool input holds newlines and a log line must stay one
/// line; cut by characters, never by bytes, so a truncated accent cannot
/// produce invalid output.
fn flat(text: &str, width: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= width {
        return joined;
    }
    let kept: String = joined.chars().take(width).collect();
    format!("{kept}…")
}

/// The scalar at `key`, as text, or `None`.
///
/// A number or a boolean is rendered rather than refused: `limit: 50` is worth
/// reading, and `Value::as_str` alone would drop it.
fn scalar(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// What a tool call is doing, in a few words.
fn call(name: &str, input: &Value) -> String {
    if let Some(said) = TELLING.iter().find_map(|key| scalar(input, key)) {
        return format!("→ {name}: {}", flat(&said, WIDTH));
    }
    match input {
        Value::Object(fields) if fields.is_empty() => format!("→ {name}"),
        Value::Null => format!("→ {name}"),
        other => format!("→ {name}: {}", flat(&other.to_string(), WIDTH)),
    }
}

/// The text of a tool result, which the API gives as a string or as blocks.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => flat(text, WIDTH),
        Value::Array(blocks) => {
            let joined = blocks
                .iter()
                .filter_map(|block| scalar(block, "text"))
                .collect::<Vec<_>>()
                .join(" ");
            flat(&joined, WIDTH)
        }
        Value::Null => String::new(),
        other => flat(&other.to_string(), WIDTH),
    }
}

/// The content blocks of a `message`, whatever the event's direction.
fn blocks(event: &Value) -> &[Value] {
    event
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// What an `assistant` event did: its prose, and each tool it called.
fn said(event: &Value) -> Vec<String> {
    blocks(event)
        .iter()
        .filter_map(|block| match scalar(block, "type")?.as_str() {
            "text" => {
                let text = flat(&scalar(block, "text")?, WIDTH);
                // An empty text block accompanies a tool call; it is not an
                // event.
                (!text.is_empty()).then(|| format!("  {text}"))
            }
            "tool_use" => Some(call(
                &scalar(block, "name").unwrap_or_else(|| "?".to_string()),
                block.get("input").unwrap_or(&Value::Null),
            )),
            "thinking" => Some("  (thinking)".to_string()),
            _ => None,
        })
        .collect()
}

/// What came back from the tools, as the `user` event reports it.
fn came_back(event: &Value) -> Vec<String> {
    blocks(event)
        .iter()
        .filter(|block| scalar(block, "type").as_deref() == Some("tool_result"))
        .map(|block| {
            let text = result_text(block.get("content").unwrap_or(&Value::Null));
            // A failing tool is the thing a human scanning this log is looking
            // for, so it says so rather than blending in.
            if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                format!("← error: {text}")
            } else if text.is_empty() {
                "← ok".to_string()
            } else {
                format!("← {text}")
            }
        })
        .collect()
}

/// The closing line of a turn: what it cost, or why it failed.
fn finished(event: &Value) -> String {
    let cost = event
        .get("total_cost_usd")
        .and_then(Value::as_f64)
        .map_or_else(|| "cost unobserved".to_string(), |usd| format!("${usd:.4}"));
    let turns = scalar(event, "num_turns").map_or_else(String::new, |n| format!(" · {n} turns"));
    if event.get("is_error").and_then(Value::as_bool) == Some(true) {
        let why = scalar(event, "result").unwrap_or_default();
        return format!("✗ failed · {cost}{turns} · {}", flat(&why, WIDTH));
    }
    format!("✓ done · {cost}{turns}")
}

/// Events that say nothing a human watching would act on, by `type` or by
/// the `subtype` of a `system` event — the CLI uses both.
///
/// `thinking_tokens` is a running counter: it arrives several times per
/// reasoning block, and seven consecutive lines of it bury the tool call that
/// follows. The `(thinking)` line from the message itself already says the
/// session is reasoning.
const COUNTERS: [&str; 1] = ["thinking_tokens"];

/// How much of the window is gone, when that is worth saying.
///
/// Silent while the status is plainly `allowed`: this event precedes every
/// API call, and repeating "3% used" would double the log for nothing. It
/// speaks once the CLI itself starts warning — which is the moment a human
/// wants to know, because what follows is a session that stops mid-task.
fn rate_limit(event: &Value) -> Option<String> {
    let info = event.get("rate_limit_info")?;
    let status = scalar(info, "status")?;
    if status == "allowed" {
        return None;
    }
    let window =
        scalar(info, "rateLimitType").unwrap_or_else(|| crate::domain::quota::UNNAMED.to_string());
    let used = info
        .get("utilization")
        .and_then(Value::as_f64)
        .map_or_else(String::new, |share| format!(" {:.0}%", share * 100.0));
    Some(format!("· rate limit {window}{used} ({status})"))
}

/// The last thing this stream said about the rate-limit windows, if anything.
///
/// The **last**, not the first: the events arrive before every API call, so the
/// final one is the only one describing the windows as a later run will find
/// them. Reading the first would record the state before this session spent
/// anything, which is precisely the number that misleads.
///
/// # The shape, as the CLI really emits it
///
/// Verified against a captured stream, not assumed — the first version of this
/// read a top-level `utilization` that does not exist, found nothing on every
/// real event, and would have shipped a gate that never fired:
///
/// ```text
/// { "type": "rate_limit_event", "rate_limit_info": {
///     "status": "allowed", "rateLimitType": "five_hour",
///     "unifiedWindows": { "five_hour": { "utilization": 0.4, "resetsAt": … },
///                         "seven_day": { "utilization": 0.7, "resetsAt": … } } } }
/// ```
///
/// A top-level `utilization` is still read as a single window, for a CLI version
/// that reports one that way. Unlike [`rate_limit`], which renders for a human,
/// this keeps `allowed` readings too: a window at 3% is exactly what a preflight
/// wants to be told, and staying quiet about it is a choice about logs, not facts.
#[must_use]
pub fn last_rate_limit(stdout: &str, at: u64) -> Option<crate::domain::quota::Reading> {
    let mut found = None;
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
            continue;
        }
        let Some(info) = event.get("rate_limit_info") else {
            continue;
        };
        let windows = windows_in(info);
        // An event naming no measurable window says nothing. Skipped rather than
        // recorded as empty, which `decide` would read as "all windows reset".
        if !windows.is_empty() {
            found = Some(crate::domain::quota::Reading { windows, at });
        }
    }
    found
}

/// Every window a `rate_limit_info` describes, nested form first.
fn windows_in(info: &Value) -> Vec<crate::domain::quota::Window> {
    use crate::domain::quota::{UNNAMED, Window};

    let resets = |at: &Value| at.get("resetsAt").and_then(Value::as_u64).unwrap_or(0);
    if let Some(nested) = info.get("unifiedWindows").and_then(Value::as_object) {
        let mut windows: Vec<Window> = nested
            .iter()
            .filter_map(|(name, window)| {
                Some(Window {
                    name: name.clone(),
                    utilization: window.get("utilization").and_then(Value::as_f64)?,
                    resets_at: resets(window),
                })
            })
            .collect();
        // `serde_json` preserves insertion order only with the `preserve_order`
        // feature; sorting by name keeps the file byte-stable between runs, which
        // is what makes a diff of it readable.
        windows.sort_by(|left, right| left.name.cmp(&right.name));
        if !windows.is_empty() {
            return windows;
        }
    }
    // The flat form: one window, named by `rateLimitType`.
    info.get("utilization")
        .and_then(Value::as_f64)
        .map(|utilization| {
            vec![Window {
                name: scalar(info, "rateLimitType").unwrap_or_else(|| UNNAMED.to_string()),
                utilization,
                resets_at: resets(info),
            }]
        })
        .unwrap_or_default()
}

/// Every line worth writing for this event, or nothing.
///
/// A line that is not JSON comes back as itself: `claude` prints warnings on
/// stdout before the stream starts, and dropping them would hide the reason a
/// turn never produced an event.
#[must_use]
pub fn render(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
        return vec![flat(trimmed, WIDTH)];
    };
    match scalar(&event, "type").unwrap_or_default().as_str() {
        "system" => {
            let subtype = scalar(&event, "subtype").unwrap_or_default();
            if COUNTERS.contains(&subtype.as_str()) {
                return Vec::new();
            }
            let model = scalar(&event, "model").map_or_else(String::new, |m| format!(" {m}"));
            vec![format!("· {subtype}{model}")]
        }
        "assistant" => said(&event),
        "user" => came_back(&event),
        "result" => vec![finished(&event)],
        "rate_limit_event" => rate_limit(&event).into_iter().collect(),
        other if COUNTERS.contains(&other) => Vec::new(),
        // A future event type is not an error: naming it is enough to show
        // that something happened, and this log is not a parser.
        other if !other.is_empty() => vec![format!("· {other}")],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_call_is_named_with_the_argument_that_says_what_it_does() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Bash","input":{"command":"npm test","timeout":9}}]}}"#;
        assert_eq!(render(line), ["→ Bash: npm test"]);
    }

    #[test]
    fn a_file_tool_shows_the_file_rather_than_its_whole_input() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Edit","input":{"file_path":"src/core/codec.ts",
             "old_string":"a very long block of code","new_string":"another one"}}]}}"#;
        assert_eq!(render(line), ["→ Edit: src/core/codec.ts"]);
    }

    #[test]
    fn a_tool_with_no_telling_field_is_still_named() {
        // Better than a line of braces, and better than silence: the call
        // happened.
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"TodoWrite","input":{}}]}}"#;
        assert_eq!(render(line), ["→ TodoWrite"]);
    }

    #[test]
    fn prose_and_a_call_in_the_same_message_are_two_lines_in_order() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"Let me run the tests."},
            {"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#;
        assert_eq!(
            render(line),
            ["  Let me run the tests.", "→ Bash: npm test"]
        );
    }

    #[test]
    fn an_empty_text_block_beside_a_call_is_not_an_event() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"  "},
            {"type":"tool_use","name":"Read","input":{"file_path":"a.ts"}}]}}"#;
        assert_eq!(render(line), ["→ Read: a.ts"]);
    }

    #[test]
    fn a_failing_tool_says_so_rather_than_blending_in() {
        // What a human scanning this log is looking for.
        let line = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","is_error":true,"content":"1 test failed"}]}}"#;
        assert_eq!(render(line), ["← error: 1 test failed"]);
    }

    #[test]
    fn a_tool_result_in_blocks_reads_like_one_in_a_string() {
        let line = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","content":[{"type":"text","text":"all green"}]}]}}"#;
        assert_eq!(render(line), ["← all green"]);
    }

    #[test]
    fn a_silent_tool_result_is_reported_as_ok_not_as_blank() {
        let line = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","content":""}]}}"#;
        assert_eq!(render(line), ["← ok"]);
    }

    #[test]
    fn the_last_line_of_a_turn_carries_what_it_cost() {
        let line = r#"{"type":"result","is_error":false,"total_cost_usd":0.4242,"num_turns":7}"#;
        assert_eq!(render(line), ["✓ done · $0.4242 · 7 turns"]);
    }

    #[test]
    fn a_failed_turn_says_why_on_its_closing_line() {
        let line = r#"{"type":"result","is_error":true,"total_cost_usd":0.0,
                       "result":"You've hit your session limit"}"#;
        assert_eq!(
            render(line),
            ["✗ failed · $0.0000 · You've hit your session limit"]
        );
    }

    #[test]
    fn an_unobserved_cost_is_said_in_words_not_shown_as_zero() {
        // Same rule as the ledger: zero would read as a free turn.
        let line = r#"{"type":"result","is_error":false}"#;
        assert_eq!(render(line), ["✓ done · cost unobserved"]);
    }

    #[test]
    fn the_init_event_names_the_model_that_will_be_billed() {
        let line = r#"{"type":"system","subtype":"init","model":"opus","tools":["Bash"]}"#;
        assert_eq!(render(line), ["· init opus"]);
    }

    #[test]
    fn a_warned_rate_limit_says_how_much_of_the_window_is_gone() {
        // The moment worth knowing: what follows is a session that stops
        // mid-task.
        let line = r#"{"type":"rate_limit_event","rate_limit_info":{
            "status":"allowed_warning","rateLimitType":"five_hour","utilization":0.97}}"#;
        assert_eq!(
            render(line),
            ["· rate limit five_hour 97% (allowed_warning)"]
        );
    }

    #[test]
    fn a_rate_limit_well_within_its_window_stays_silent() {
        // It precedes every API call; repeating "3% used" would double the
        // log for nothing.
        let line = r#"{"type":"rate_limit_event","rate_limit_info":{
            "status":"allowed","rateLimitType":"five_hour","utilization":0.03}}"#;
        assert!(render(line).is_empty());
    }

    /// A real event, with the nested shape the CLI actually emits.
    fn nested(five: f64, seven: f64, resets: u64) -> String {
        format!(
            r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","rateLimitType":"five_hour","resetsAt":{resets},"unifiedWindows":{{"five_hour":{{"utilization":{five},"resetsAt":{resets}}},"seven_day":{{"utilization":{seven},"resetsAt":{resets}}}}}}}}}"#
        )
    }

    #[test]
    fn the_real_nested_shape_gives_both_windows() {
        // Captured from a live run. The first version of this reader looked for a
        // top-level `utilization`, which does not exist here: it found nothing on
        // every real event, and would have shipped a gate that never fired.
        let found = last_rate_limit(&nested(0.4, 0.7, 1_791_352_800), 42).expect("a reading");
        assert_eq!(found.windows.len(), 2, "{found:?}");
        let five = &found.windows[0];
        assert_eq!(five.name, "five_hour");
        assert_eq!(five.left_percent(), 60);
        assert_eq!(five.resets_at, 1_791_352_800);
        // Sorted by name, so the recorded file is byte-stable between runs.
        assert_eq!(found.windows[1].name, "seven_day");
        assert_eq!(found.windows[1].left_percent(), 30);
        assert_eq!(found.at, 42);
    }

    #[test]
    fn the_last_rate_limit_of_a_stream_is_the_one_kept() {
        // The events precede every API call, so the final one is the only one
        // describing the windows as a later run will find them.
        let stream = format!(
            "{}\n{}\n{}",
            nested(0.10, 0.10, 9),
            r#"{"type":"assistant","message":{"content":[]}}"#,
            nested(0.93, 0.20, 9),
        );
        let found = last_rate_limit(&stream, 1).expect("a reading");
        assert_eq!(found.windows[0].left_percent(), 7, "{found:?}");
    }

    #[test]
    fn the_flat_shape_of_an_older_cli_is_still_read_as_one_window() {
        let line = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"five_hour","utilization":0.97}}"#;
        let found = last_rate_limit(line, 1).expect("a reading");
        assert_eq!(found.windows.len(), 1);
        assert_eq!(found.windows[0].name, "five_hour");
        assert_eq!(found.windows[0].left_percent(), 3);
        // No `resetsAt` in this form: `0` means "unknown", never "already reset".
        assert_eq!(found.windows[0].resets_at, 0);
    }

    #[test]
    fn an_allowed_reading_is_recorded_even_though_it_logs_nothing() {
        // `render` stays silent on `allowed` to keep the journal readable. A
        // preflight needs the number anyway: "3% used" is the answer it wants.
        let line = nested(0.03, 0.03, 9);
        assert!(render(&line).is_empty(), "still silent in the journal");
        assert!(last_rate_limit(&line, 1).is_some(), "but recorded");
    }

    #[test]
    fn a_stream_without_any_rate_limit_event_reads_as_no_reading() {
        let stream = r#"{"type":"assistant","message":{"content":[]}}"#;
        assert!(last_rate_limit(stream, 1).is_none());
    }

    #[test]
    fn an_event_naming_no_measurable_window_is_skipped() {
        // Recorded as empty, `decide` would read it as "all windows reset" and
        // start — the one answer that lets a run begin when it should not.
        let line = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour"}}"#;
        assert!(last_rate_limit(line, 1).is_none());
    }

    #[test]
    fn a_line_that_is_not_json_is_kept_as_itself() {
        // `claude` prints warnings before the stream starts, and dropping them
        // would hide why a turn produced no event at all.
        assert_eq!(
            render("warning: config file is unreadable"),
            ["warning: config file is unreadable"]
        );
    }

    #[test]
    fn an_empty_line_produces_nothing() {
        assert!(render("").is_empty());
        assert!(render("   \n").is_empty());
    }

    #[test]
    fn a_running_counter_does_not_bury_the_call_that_follows_it() {
        // It arrives several times per reasoning block; `(thinking)` already
        // says the session is reasoning. The CLI sends it as a `system`
        // subtype, which is where the first attempt at this filter missed it.
        assert!(render(r#"{"type":"system","subtype":"thinking_tokens","count":12}"#).is_empty());
        assert!(render(r#"{"type":"thinking_tokens","count":12}"#).is_empty());
    }

    #[test]
    fn the_init_event_is_not_swallowed_with_the_counters() {
        assert_eq!(
            render(r#"{"type":"system","subtype":"init","model":"opus"}"#),
            ["· init opus"]
        );
    }

    #[test]
    fn an_unknown_event_type_is_named_rather_than_dropped_or_parsed() {
        assert_eq!(
            render(r#"{"type":"stream_event","x":1}"#),
            ["· stream_event"]
        );
    }

    #[test]
    fn a_long_value_is_cut_on_a_character_boundary() {
        let long = "é".repeat(WIDTH + 50);
        let line = format!(
            r#"{{"type":"assistant","message":{{"content":[
                {{"type":"tool_use","name":"Bash","input":{{"command":"{long}"}}}}]}}}}"#
        );
        let said = render(&line);
        assert_eq!(said.len(), 1);
        assert!(said[0].ends_with('…'));
        // The cut counts characters, so a two-byte accent cannot be halved.
        assert_eq!(said[0].chars().filter(|c| *c == 'é').count(), WIDTH);
    }

    #[test]
    fn a_newline_inside_a_tool_input_cannot_break_the_line_in_two() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Bash","input":{"command":"a\nb\nc"}}]}}"#;
        let said = render(line);
        assert_eq!(said, ["→ Bash: a b c"]);
        assert!(!said[0].contains('\n'));
    }
}

//! The design system and its kit, as the second easel of the office shows
//! them: the tokens injected or generated, validated on a sheet; the kit's
//! catalogue ticked and prompted; the components on a canvas, edited by
//! request pins, saved as a fork or a replacement. The shapes are
//! `harness_architecture::kit`'s (contract O); this module writes the
//! prompts, reads the answers, and composes what the page shows. Pure: the
//! `KitDesk` does the reading, the writing and the sessions.

use std::fmt::Write as _;

use harness_architecture::kit::{self, Base, Component, ComponentState, Design, Kit, Token};
use serde::Serialize;

use crate::domain::annotations::{Kind, Note};
use crate::domain::drafts::Asked;

/// The pin board of the kit's canvas: one for the product, every component
/// on it.
pub const BOARD: &str = "kit--components";

/// `mock/design/design.json`, relative to `mock/`.
pub const DESIGN_JSON: &str = "design/design.json";
/// The validated tokens, relative to `mock/`.
pub const TOKENS_JSON: &str = "design/tokens.json";
/// The derived stylesheet, relative to `mock/`.
pub const TOKENS_CSS: &str = "design/tokens.css";
/// A candidate waiting beside the validated tokens, relative to `mock/`.
pub const CANDIDATE_JSON: &str = "design/candidate.json";
/// `mock/kit/kit.json`, relative to `mock/`.
pub const KIT_JSON: &str = "kit/kit.json";

/// The longest brief for a generation, in characters.
pub const MAX_BRIEF: usize = 4_000;
/// The longest prompt for a custom component, in characters.
pub const MAX_PROMPT: usize = 4_000;
/// The heaviest image the easel takes, in bytes.
pub const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;
/// The most images a moodboard holds.
pub const MAX_MOODBOARD: usize = 12;

/// The image types the easel takes, by extension.
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "svg"];

/// A file name an image may have on the easel: plain, with an image
/// extension, no path.
#[must_use]
pub fn is_asset_name(name: &str) -> bool {
    let plain = (1..=80).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        && !name.starts_with('.');
    plain
        && name.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty() && IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        })
}

/// A file name made plain for the easel: lower case, the characters a name
/// may have, the extension kept.
#[must_use]
pub fn plain_asset_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "png"));
    let mut plain = String::new();
    for c in stem.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            plain.push(c);
        } else if !plain.ends_with('-') && !plain.is_empty() {
            plain.push('-');
        }
    }
    plain = plain.trim_matches('-').chars().take(60).collect();
    if plain.is_empty() {
        plain = "image".to_string();
    }
    format!("{plain}.{}", ext.to_ascii_lowercase())
}

/// What the page gets of the design system: the record, the tokens, the
/// candidate's, where the branch stands.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DesignView {
    /// The record.
    pub design: Design,
    /// The validated tokens, flattened.
    pub tokens: Vec<Token>,
    /// The tokens file's text, to edit.
    pub tokens_json: String,
    /// The candidate's tokens, when one waits.
    pub candidate: Vec<Token>,
    /// The candidate's file's text.
    pub candidate_json: String,
    /// The derived stylesheet.
    pub tokens_css: String,
    /// The tokens of the kit's vocabulary the file leaves out.
    pub missing: Vec<&'static str>,
}

/// One component of the kit as the catalogue and the canvas list it: the
/// base's description or the custom prompt, where it stands, what the
/// static check says of its fragment.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Entry {
    /// `Carousel`.
    pub name: String,
    /// The base's id, or empty for a custom one.
    pub base: String,
    /// For a human.
    pub title: String,
    /// What it shows and does.
    pub what: String,
    /// The human's prompt.
    pub prompt: String,
    /// A reference image, relative to `mock/kit/`, or empty.
    pub reference: String,
    /// Where it stands.
    pub state: ComponentState,
    /// Why the last generation failed.
    pub why: String,
    /// Forked from.
    pub variant_of: String,
    /// What the code stage changed, one line.
    pub recoded: String,
    /// When the fragment was last written.
    pub generated_at: String,
    /// The base is ticked (a base entry), or the component is listed (a
    /// custom one — the one with no `base`).
    pub checked: bool,
    /// The fragment is there.
    pub fragment: bool,
    /// What the static check says of the fragment; empty passes.
    pub checks: Vec<String>,
    /// A draft of the fragment waits, from the canvas's requests.
    pub draft: bool,
}

/// The catalogue: every base, ticked or not, merged with the kit's
/// components; the custom ones after. `fragment_of` answers a component's
/// fragment when it is there, `draft_of` whether a draft waits.
pub fn catalog(
    kit: &Kit,
    fragment_of: impl Fn(&str) -> Option<String>,
    draft_of: impl Fn(&str) -> bool,
) -> Vec<Entry> {
    let mut entries = Vec::new();
    let entry_of = |c: &Component, base: Option<&Base>| {
        let fragment = fragment_of(&c.name);
        Entry {
            name: c.name.clone(),
            base: c.base.clone(),
            title: if c.title.is_empty() {
                base.map_or_else(|| c.name.clone(), |b| b.title.to_string())
            } else {
                c.title.clone()
            },
            what: if c.what.is_empty() {
                base.map_or_else(String::new, |b| b.what.to_string())
            } else {
                c.what.clone()
            },
            prompt: c.prompt.clone(),
            reference: c.reference.clone(),
            state: c.state,
            why: c.why.clone(),
            variant_of: c.variant_of.clone(),
            recoded: c.recoded.clone(),
            generated_at: c.generated_at.clone(),
            checked: c.base.is_empty() || kit.checked.contains(&c.base),
            checks: fragment.as_deref().map(kit::check).unwrap_or_default(),
            fragment: fragment.is_some(),
            draft: draft_of(&c.name),
        }
    };
    for base in &kit::BASE {
        if let Some(c) = kit.components.iter().find(|c| c.base == base.id) {
            entries.push(entry_of(c, Some(base)));
        } else {
            entries.push(Entry {
                name: base.name.to_string(),
                base: base.id.to_string(),
                title: base.title.to_string(),
                what: base.what.to_string(),
                checked: kit.checked.iter().any(|id| id == base.id),
                ..Entry::default()
            });
        }
    }
    for c in kit
        .components
        .iter()
        .filter(|c| c.base.is_empty() || kit::base(&c.base).is_none())
    {
        entries.push(entry_of(c, None));
    }
    entries
}

/// The prompt that draws a design system from a moodboard and a logo: the
/// brief, the files to read, the tokens to answer and their format.
#[must_use]
pub fn generate_prompt(brief: &str, moodboard: &[String], logo: &str) -> String {
    let mut text = String::from(
        "You are a brand designer. Draw a design system — a set of design tokens — for a \
         product's web interface, from the material below. Look at every image listed with \
         your Read tool before answering: the moodboard says the mood, the palette and the \
         type the human wants; the logo fixes the primary colour and what it must contrast \
         with. Choose real, accessible values: text on surface at 4.5:1 or better, primary \
         against its contrast colour at 4.5:1 or better, a font stack of common web-safe \
         or system fonts (name a webfont only if the moodboard clearly shows one, with \
         system fallbacks after it).\n\n",
    );
    if !brief.trim().is_empty() {
        let _ = writeln!(text, "The human's brief:\n{}\n", brief.trim());
    }
    if moodboard.is_empty() && logo.is_empty() {
        text.push_str("No image was given: draw from the brief alone.\n");
    }
    if !moodboard.is_empty() {
        let _ = writeln!(
            text,
            "The moodboard, {} image(s), relative to the folder you are in:",
            moodboard.len()
        );
        for image in moodboard {
            let _ = writeln!(text, "- {image}");
        }
    }
    if !logo.is_empty() {
        let _ = writeln!(text, "The logo: {logo}");
    }
    text.push_str(
        "\nAnswer ONLY a JSON object in the W3C design tokens format — no prose, no code \
         fence — carrying exactly these tokens (their `$type` as given; every `$value` a \
         CSS value: a colour as `#rrggbb`, a dimension with its unit, a font family as a \
         comma-separated stack, a shadow as the CSS text):\n",
    );
    for spec in &kit::TOKENS {
        let _ = writeln!(text, "- {} ({})", spec.path, spec.kind);
    }
    text.push_str(
        "\nShaped like {\"color\": {\"primary\": {\"$type\": \"color\", \"$value\": \"#…\"}, …}, \
         \"font\": {…}, \"space\": {…}, \"radius\": {…}, \"shadow\": {…}, \"border\": {…}}. \
         You may add a few tokens of your own under the same groups when the moodboard calls \
         for them (a second accent, a display font), named in kebab-case.",
    );
    text
}

/// The tokens an answer carries: the JSON object in it, completed with the
/// vocabulary's fallbacks for what it leaves out. Answers the file's text
/// and what was added.
///
/// # Errors
///
/// No object in the answer, or not a tokens file.
pub fn tokens_from_answer(answer: &str) -> Result<(String, Vec<&'static str>), String> {
    let start = answer.find('{').ok_or("the answer carries no tokens")?;
    let end = answer.rfind('}').ok_or("the answer carries no tokens")?;
    if end < start {
        return Err("the answer carries no tokens".to_string());
    }
    let text = answer.get(start..=end).unwrap_or_default();
    let (done, added) = kit::completed(text)?;
    if added.len() == kit::TOKENS.len() {
        return Err("the answer names none of the kit's tokens".to_string());
    }
    Ok((done, added))
}

/// The prompt that styles one component with the tokens: the base fragment
/// to start from (or nothing, for a custom one), the tokens, the rules a
/// fragment keeps, and the shape of the answer.
#[must_use]
pub fn component_prompt(component: &Component, base: Option<&Base>, tokens_css: &str) -> String {
    let mut text = format!(
        "You make one component of a product's kit of visual components, `{name}`: {title} — \
         {what}. A kit component is a visual primitive for the product's mock-up and, when it \
         passes the checks below, for its real frontend: markup, a `<style>` block and, only \
         for a visual dynamic (a carousel turning, a menu opening, tabs switching), a \
         `<script>`. It has no data of its own and never reaches a server: what loads from a \
         server is a micro-frontend, not a kit component.\n\n\
         Rules, checked by a static analysis before the real code reuses it:\n\
         - every colour, font, spacing, radius and shadow comes from the design system's \
         variables: `var(--color-primary, #2b6cb0)` with a fallback, never a bare literal \
         colour in a style — the variables are below, consume them by name;\n\
         - no `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, no dynamic `import`, \
         no `<script src>`, no `<link>`, no `@import`, no `<iframe>`;\n\
         - the root element carries the class `kit-{lower}`, and every selector of the style \
         block is scoped under it, so two components on one page never collide;\n\
         - no `data-kit` attribute anywhere in the fragment: the page's host element carries \
         it, and the loader fills every `data-kit` it meets;\n\
         - a script finds its root with `document.currentScript.closest('[data-kit]') || \
         document.currentScript.parentElement` and touches nothing outside it;\n\
         - example content is real and short (names, amounts, labels), never lorem ipsum;\n\
         - under {max} characters.\n\n",
        name = component.name,
        title = component.title,
        what = component.what,
        lower = component.name.to_ascii_lowercase(),
        max = kit::MAX_FRAGMENT,
    );
    if !component.prompt.trim().is_empty() {
        let _ = writeln!(
            text,
            "What the human asks of it:\n{}\n",
            component.prompt.trim()
        );
    }
    if !component.reference.is_empty() {
        let _ = writeln!(
            text,
            "A reference image is beside you: `{}`. Read it with your Read tool and reproduce \
             its layout and feel with the tokens.\n",
            component.reference
        );
    }
    let _ = writeln!(
        text,
        "The design system's variables (mock/design/tokens.css):\n```css\n{}\n```",
        tokens_css.trim()
    );
    if let Some(base) = base {
        let _ = writeln!(
            text,
            "\nStart from this neutral base — keep its structure, its classes and its script, \
             and restyle it with the tokens: the shapes (radii, borders, shadows, spacing) \
             and the type the design system implies, not just the colours. Change the \
             markup only where the design system calls for it.\n```html\n{}\n```",
            base.fragment.trim()
        );
    } else {
        text.push_str(
            "\nThere is no base for it: draw it from the human's words, in the manner of the \
             design system.\n",
        );
    }
    text.push_str(
        "\nAnswer ONLY the fragment — the HTML, with its `<style>` and its `<script>` when \
         it has one — no prose, no code fence.",
    );
    text
}

/// The prompt that applies requests to one component's fragment: the
/// fragment as it is, the tokens, the requests, the same rules, and the
/// whole fragment back.
#[must_use]
pub fn rewrite_prompt(name: &str, fragment: &str, tokens_css: &str, asked: &[Asked]) -> String {
    let requests = asked
        .iter()
        .map(|a| format!("- request `{}`: {}", a.id, a.text.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You edit one component of a product's kit of visual components, `{name}`, on behalf \
         of a human who dropped requests on it. Apply every request to the fragment below and \
         answer the WHOLE fragment back — the HTML with its `<style>` and `<script>` — no \
         prose, no code fence. Keep what the requests do not touch as it is. The rules a \
         fragment keeps: every colour, font, spacing, radius and shadow through the design \
         system's variables (`var(--color-primary, #2b6cb0)`, a fallback, never a bare \
         literal colour in a style); no network (`fetch`, `XMLHttpRequest`, `WebSocket`, \
         `EventSource`, a beacon, a dynamic `import`), no `<script src>`, no `<link>`, no \
         `@import`, no `<iframe>`; a script finds its root with \
         `document.currentScript.closest('[data-kit]') || document.currentScript.parentElement`; \
         no `data-kit` attribute in the fragment (the host element carries it); \
         a visual dynamic only, no data of its own; under {max} characters. A request that asks \
         for data from a server, or for behaviour beyond the visual, is not a kit matter: \
         leave the fragment unchanged for it and say so in one line at the very end, after the \
         fragment, as `<!-- skipped <id>: <why> -->`.\n\n\
         The design system's variables:\n```css\n{css}\n```\n\n\
         The requests:\n{requests}\n\n<fragment>\n{fragment}\n</fragment>",
        max = kit::MAX_FRAGMENT,
        css = tokens_css.trim(),
    )
}

/// The fragment an answer carries: the text, a code fence stripped, that
/// holds markup and passes the weight limit.
///
/// # Errors
///
/// No markup in the answer, or too heavy.
pub fn fragment_from_answer(answer: &str) -> Result<String, String> {
    let mut text = answer.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or("", |(_, body)| body);
        text = rest.trim_end().strip_suffix("```").unwrap_or(rest).trim();
    }
    if !text.contains('<') || !text.contains('>') {
        return Err("the answer carries no markup".to_string());
    }
    if text.chars().count() > kit::MAX_FRAGMENT {
        return Err(format!(
            "the fragment weighs more than {} characters",
            kit::MAX_FRAGMENT
        ));
    }
    Ok(format!("{text}\n"))
}

/// What a rewrite said it skipped: `<!-- skipped r1: why -->` lines, as
/// `(id, why)`.
#[must_use]
pub fn skipped_in(fragment: &str) -> Vec<(String, String)> {
    fragment
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let inner = line.strip_prefix("<!-- skipped ")?.strip_suffix("-->")?;
            let (id, why) = inner.split_once(':')?;
            Some((id.trim().to_string(), why.trim().to_string()))
        })
        .collect()
}

/// The roadmap task a validated design system opens: install the tokens in
/// the product's real frontend, so the first screen built does not reinvent
/// them. `(title, body)`.
#[must_use]
pub fn install_issue(design: &Design, tokens: &[Token]) -> (String, String) {
    let title = "Install the design system in the product's frontend".to_string();
    let mut body = String::from(
        "The design system was validated on the factory view's easel. Its tokens live on the \
         `design-system` branch, `mock/design/tokens.json` (W3C design tokens), derived into \
         `mock/design/tokens.css` — one CSS variable per token. Install them in the product's \
         real frontend: the variables loaded once by the shell (or the screen host), every \
         screen and Micro-UI styled through them and never through a literal colour, font, \
         spacing, radius or shadow; the kit's components (`mock/kit/<Name>.html`, injected \
         with `data-kit`) reused as they are when they pass the kit's static check.\n\n",
    );
    let _ = writeln!(
        body,
        "Source: {}{}",
        match design.source {
            kit::Source::Generated => "generated from the moodboard and the logo",
            kit::Source::Injected => "a tokens file the human handed over",
            kit::Source::None => "none",
        },
        if design.brief.trim().is_empty() {
            String::new()
        } else {
            format!(" — brief: {}", design.brief.trim())
        }
    );
    let _ = writeln!(body, "\nThe tokens ({}):\n", tokens.len());
    for token in tokens.iter().take(80) {
        let _ = writeln!(
            body,
            "- `{}`: `{}`",
            kit::variable_of(&token.path),
            token.value
        );
    }
    body.push_str(
        "\nAcceptance: every screen of the product reads its colours, fonts, spacings, radii \
         and shadows from these variables; changing a token in `mock/design/tokens.json` and \
         rebuilding restyles the product with no other change.\n",
    );
    (title, body)
}

/// The kit as the request AI reads it: one line per component with a
/// fragment, name and what it is.
#[must_use]
pub fn kit_summary(kit: &Kit) -> String {
    let lines: Vec<String> = kit
        .components
        .iter()
        .filter(|c| c.state == ComponentState::Generated)
        .map(|c| {
            let base = kit::base(&c.base);
            format!(
                "- {} ({}): {}",
                c.name,
                if c.title.is_empty() {
                    base.map_or("custom", |b| b.title)
                } else {
                    &c.title
                },
                if c.what.is_empty() {
                    base.map_or(c.prompt.as_str(), |b| b.what)
                } else {
                    &c.what
                }
            )
        })
        .collect();
    if lines.is_empty() {
        "(no component generated yet)".to_string()
    } else {
        lines.join("\n")
    }
}

/// Where a kit component is used: a page or a micro-frontend of a branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Usage {
    /// The branch.
    pub branch: String,
    /// `page` or `micro-frontend`.
    pub kind: &'static str,
    /// The page's path, or the micro-frontend's name.
    pub what: String,
    /// The page's title, or empty.
    pub title: String,
}

/// A branch the easel counts as active when it asks who uses a component:
/// not a saved mock-up, not the design system's own, not the data archive.
#[must_use]
pub fn is_active_branch(branch: &str) -> bool {
    !branch.starts_with("mockup/")
        && branch != kit::DESIGN_BRANCH
        && branch != harness_workflows::data_snapshot::config::ARCHIVE_BRANCH
}

/// The request pin the code stage's recoding leaves on the canvas: on the
/// component's card, what changed.
#[must_use]
pub fn recoded_pin(component: &Component, at: &str) -> Note {
    Note {
        id: format!("recoded-{}", component.name.to_ascii_lowercase()),
        target: format!("kit:{}", component.name),
        text: format!(
            "Recoded by the code stage: {}\nThe real frontend changed this component; the fragment was rewritten to match it.",
            component.recoded.trim()
        ),
        at: at.to_string(),
        kind: Kind::Request,
        ..Note::default()
    }
}

/// A name for a fork of `from`: `<from>2`, `<from>3`… the first the kit does
/// not hold.
#[must_use]
pub fn fork_name(kit: &Kit, from: &str) -> String {
    (2..100)
        .map(|n| format!("{from}{n}"))
        .find(|name| kit.component(name).is_none())
        .unwrap_or_else(|| format!("{from}Copy"))
}

/// The components still to generate among `names` (every pending or
/// failed one when `names` is empty), in the catalogue's order.
#[must_use]
pub fn to_generate(kit: &Kit, names: &[String]) -> Vec<String> {
    kit.components
        .iter()
        .filter(|c| {
            if names.is_empty() {
                matches!(c.state, ComponentState::Pending | ComponentState::Failed)
            } else {
                names.contains(&c.name)
            }
        })
        .filter(|c| c.state != ComponentState::Running)
        .map(|c| c.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn an_asset_name_is_plain_and_an_image() {
        assert!(is_asset_name("mood-1.png"));
        assert!(is_asset_name("Logo.SVG"));
        assert!(!is_asset_name("../x.png"));
        assert!(!is_asset_name("x.exe"));
        assert!(!is_asset_name(".png"));
        assert_eq!(plain_asset_name("My Board (1).PNG"), "my-board-1.png");
        assert_eq!(plain_asset_name("/tmp/a b.jpeg"), "a-b.jpeg");
        assert_eq!(plain_asset_name("!!!"), "image.png");
    }

    #[test]
    fn the_generation_prompt_names_the_images_and_the_tokens() {
        let prompt = generate_prompt("warm, artisanal", &["moodboard/a.png".into()], "logo.svg");
        assert!(prompt.contains("- moodboard/a.png"));
        assert!(prompt.contains("The logo: logo.svg"));
        assert!(prompt.contains("- color.primary (color)"));
        assert!(prompt.contains("warm, artisanal"));
        assert!(generate_prompt("", &[], "").contains("No image was given"));
        let answer = format!("Here:\n{}", kit::default_tokens_json());
        let (json, added) = tokens_from_answer(&answer).expect("read");
        assert!(added.is_empty());
        assert!(kit::tokens_of(&json).is_ok());
        assert!(tokens_from_answer("nothing").is_err());
        assert!(
            tokens_from_answer("{\"color\":{}}").is_err(),
            "too few tokens"
        );
    }

    #[test]
    fn a_component_prompt_starts_from_the_base_and_an_answer_is_a_fragment() {
        let base = kit::base("button").expect("button");
        let c = Component {
            name: "Button".into(),
            base: "button".into(),
            title: base.title.into(),
            what: base.what.into(),
            prompt: "rounder".into(),
            ..Component::default()
        };
        let prompt = component_prompt(&c, Some(base), ":root { --color-primary: #1; }");
        assert!(prompt.contains("kit-button"));
        assert!(prompt.contains("rounder"));
        assert!(prompt.contains("Start from this neutral base"));
        assert!(prompt.contains("--color-primary: #1"));
        let custom = Component {
            name: "Hero".into(),
            reference: "Hero.reference.png".into(),
            ..Component::default()
        };
        let prompt = component_prompt(&custom, None, "");
        assert!(prompt.contains("no base"));
        assert!(prompt.contains("Hero.reference.png"));
        assert_eq!(
            fragment_from_answer("```html\n<div class=\"kit-x\">hi</div>\n```").expect("ok"),
            "<div class=\"kit-x\">hi</div>\n"
        );
        assert!(fragment_from_answer("no markup here").is_err());
        let asked = vec![Asked {
            id: "r1".into(),
            text: "bigger".into(),
            target: String::new(),
        }];
        let rewrite = rewrite_prompt("Button", "<div></div>", "", &asked);
        assert!(rewrite.contains("request `r1`: bigger"));
        assert_eq!(
            skipped_in("<div></div>\n<!-- skipped r1: needs data -->"),
            [("r1".to_string(), "needs data".to_string())]
        );
    }

    #[test]
    fn the_catalogue_merges_the_bases_and_the_kit_and_the_rest_composes() {
        let mut kit = Kit::default();
        kit.settle();
        kit.components.push(Component {
            name: "Hero".into(),
            prompt: "a big banner".into(),
            state: ComponentState::Generated,
            ..Component::default()
        });
        kit.component_mut("Button").expect("button").state = ComponentState::Generated;
        let entries = catalog(
            &kit,
            |name| (name == "Button").then(|| "<div style=\"color:#fff\">x</div>".to_string()),
            |name| name == "Hero",
        );
        assert_eq!(entries.len(), kit::BASE.len() + 1);
        let button = entries.iter().find(|e| e.name == "Button").expect("button");
        assert!(button.fragment && button.checked && !button.base.is_empty());
        assert!(
            !button.checks.is_empty(),
            "a literal colour fails the check"
        );
        let carousel = entries
            .iter()
            .find(|e| e.name == "Carousel")
            .expect("carousel");
        assert!(!carousel.checked && !carousel.fragment);
        let hero = entries.last().expect("hero");
        assert!(hero.base.is_empty() && hero.checked && hero.draft);
        assert_eq!(hero.what, "", "a custom one has no base's description");
        let summary = kit_summary(&kit);
        assert!(summary.contains("- Button (Button):"));
        assert!(summary.contains("- Hero (custom): a big banner"));
        assert!(!summary.contains("Carousel"));
        assert_eq!(fork_name(&kit, "Button"), "Button2");
        assert_eq!(to_generate(&kit, &[]).len(), kit::DEFAULT_CHECKED.len() - 1);
        assert_eq!(to_generate(&kit, &["Button".into()]), ["Button"]);
        let (title, body) = install_issue(
            &Design {
                source: kit::Source::Generated,
                brief: "warm".into(),
                ..Design::default()
            },
            &kit::tokens_of(&kit::default_tokens_json()).expect("tokens"),
        );
        assert!(title.contains("design system"));
        assert!(body.contains("brief: warm") && body.contains("`--color-primary`"));
        assert!(is_active_branch("main") && is_active_branch("milestone/3-x"));
        assert!(!is_active_branch("mockup/pages") && !is_active_branch("design-system"));
        let pin = recoded_pin(
            &Component {
                name: "Card".into(),
                recoded: "the shadow is the shell's".into(),
                ..Component::default()
            },
            "t",
        );
        assert_eq!(pin.id, "recoded-card");
        assert_eq!(pin.target, "kit:Card");
        assert!(crate::domain::annotations::checked(pin).is_ok());
    }

    proptest! {
        #[test]
        fn a_plain_asset_name_is_always_an_asset_name(name in "\\PC{1,80}", ext in "(png|jpg|svg)") {
            let plain = plain_asset_name(&format!("{name}.{ext}"));
            prop_assert!(is_asset_name(&plain), "{plain}");
        }
    }
}

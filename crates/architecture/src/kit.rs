//! The product's **design system** and its **kit** of visual components, as
//! every product the harness builds carries them, on one branch of their
//! own: `design-system` (contract O).
//!
//! The design system is a set of **tokens** — colours, fonts, spacings,
//! radii, shadows — written once in `mock/design/tokens.json` (the W3C
//! design tokens format) and derived into `mock/design/tokens.css`, one CSS
//! variable per token. Everything visual consumes the tokens **through the
//! variables**, so a token changed is a product restyled without a line
//! generated again.
//!
//! The kit is a catalogue of visual **primitives** below the Micro-UI — a
//! button, a menu, a carousel — each a fragment under `mock/kit/<Name>.html`
//! that a page or a micro-frontend injects with `<div data-kit="<Name>">`.
//! A kit component has no behaviour beyond the visual (a carousel turns, a
//! menu opens) and no data of its own: what loads from a server is a
//! micro-frontend. The harness ships a **base** of neutral fragments
//! ([`BASE`]); the generation applies the product's tokens to a base rather
//! than drawing from nothing, which is what keeps it cheap.
//!
//! [`check`] is the static analysis the code stage reuses a fragment on: no
//! network, no external script or style, colours only through the
//! variables. Pure: the files are read and written elsewhere.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The branch the design system and the kit live on, in every product.
pub const DESIGN_BRANCH: &str = "design-system";

/// Where the design system's files sit, relative to the repository.
pub const DESIGN_DIR: &str = "mock/design";

/// Where the kit's files sit, relative to the repository.
pub const KIT_DIR: &str = "mock/kit";

/// Where the kit's stylesheet sits, relative to `mock/`: every fragment's
/// `<style>` gathered, derived on publish ([`stylesheet`]).
pub const KIT_CSS: &str = "kit/kit.css";

/// The folders of `mock/` the design system owns: written on the
/// `design-system` branch only, carried onto the integration branch whole.
pub const OWNED: [&str; 2] = ["design", "kit"];

/// The most a fragment may weigh, in characters.
pub const MAX_FRAGMENT: usize = 60_000;

/// One token the kit's fragments may consume: its path in the tokens file
/// (`color.primary` — the variable is `--color-primary`), its type and the
/// neutral value the base fragments fall back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpec {
    /// `color.primary`.
    pub path: &'static str,
    /// `color`, `fontFamily`, `dimension`, `shadow`, `fontWeight`.
    pub kind: &'static str,
    /// What the base fragments fall back on without a design system.
    pub fallback: &'static str,
}

/// Every token a fragment may consume. The generation fills exactly these;
/// a tokens file may carry more, and the extra ones become variables too.
pub const TOKENS: [TokenSpec; 30] = [
    TokenSpec {
        path: "color.primary",
        kind: "color",
        fallback: "#2b6cb0",
    },
    TokenSpec {
        path: "color.primary-contrast",
        kind: "color",
        fallback: "#ffffff",
    },
    TokenSpec {
        path: "color.secondary",
        kind: "color",
        fallback: "#6b7280",
    },
    TokenSpec {
        path: "color.accent",
        kind: "color",
        fallback: "#d97706",
    },
    TokenSpec {
        path: "color.surface",
        kind: "color",
        fallback: "#ffffff",
    },
    TokenSpec {
        path: "color.surface-2",
        kind: "color",
        fallback: "#f3f4f6",
    },
    TokenSpec {
        path: "color.text",
        kind: "color",
        fallback: "#111827",
    },
    TokenSpec {
        path: "color.text-muted",
        kind: "color",
        fallback: "#6b7280",
    },
    TokenSpec {
        path: "color.border",
        kind: "color",
        fallback: "#d1d5db",
    },
    TokenSpec {
        path: "color.danger",
        kind: "color",
        fallback: "#b91c1c",
    },
    TokenSpec {
        path: "color.success",
        kind: "color",
        fallback: "#15803d",
    },
    TokenSpec {
        path: "font.family",
        kind: "fontFamily",
        fallback: "system-ui, sans-serif",
    },
    TokenSpec {
        path: "font.family-heading",
        kind: "fontFamily",
        fallback: "system-ui, sans-serif",
    },
    TokenSpec {
        path: "font.size-small",
        kind: "dimension",
        fallback: "12px",
    },
    TokenSpec {
        path: "font.size-base",
        kind: "dimension",
        fallback: "15px",
    },
    TokenSpec {
        path: "font.size-large",
        kind: "dimension",
        fallback: "20px",
    },
    TokenSpec {
        path: "font.size-heading",
        kind: "dimension",
        fallback: "28px",
    },
    TokenSpec {
        path: "font.weight-heading",
        kind: "fontWeight",
        fallback: "700",
    },
    TokenSpec {
        path: "space.1",
        kind: "dimension",
        fallback: "4px",
    },
    TokenSpec {
        path: "space.2",
        kind: "dimension",
        fallback: "8px",
    },
    TokenSpec {
        path: "space.3",
        kind: "dimension",
        fallback: "16px",
    },
    TokenSpec {
        path: "space.4",
        kind: "dimension",
        fallback: "24px",
    },
    TokenSpec {
        path: "space.5",
        kind: "dimension",
        fallback: "40px",
    },
    TokenSpec {
        path: "radius.small",
        kind: "dimension",
        fallback: "4px",
    },
    TokenSpec {
        path: "radius.medium",
        kind: "dimension",
        fallback: "8px",
    },
    TokenSpec {
        path: "radius.large",
        kind: "dimension",
        fallback: "16px",
    },
    TokenSpec {
        path: "radius.pill",
        kind: "dimension",
        fallback: "999px",
    },
    TokenSpec {
        path: "shadow.low",
        kind: "shadow",
        fallback: "0 1px 3px rgba(0, 0, 0, 0.12)",
    },
    TokenSpec {
        path: "shadow.high",
        kind: "shadow",
        fallback: "0 8px 24px rgba(0, 0, 0, 0.18)",
    },
    TokenSpec {
        path: "border.width",
        kind: "dimension",
        fallback: "1px",
    },
];

/// The variable a token path names: `color.primary` is `--color-primary`.
#[must_use]
pub fn variable_of(path: &str) -> String {
    format!("--{}", path.replace('.', "-"))
}

/// One token read from a tokens file: its path, its type and its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Token {
    /// `color.primary`.
    pub path: String,
    /// `color`, `dimension`… — the `$type` of the token or its group, or
    /// empty.
    pub kind: String,
    /// The `$value`, as text.
    pub value: String,
}

/// The tokens a W3C design tokens document holds, flattened.
///
/// Every object carrying a `$value` is a token, named by its path from the
/// root, groups joined by dots. A `$type` on a group applies to the tokens
/// below it.
///
/// # Errors
///
/// The text is not a JSON object.
pub fn tokens_of(json: &str) -> Result<Vec<Token>, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("tokens.json: not JSON — {e}"))?;
    let Some(object) = root.as_object() else {
        return Err("tokens.json: not an object of tokens".to_string());
    };
    let mut found = Vec::new();
    walk(object, "", "", &mut found);
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

fn walk(group: &serde_json::Map<String, Value>, prefix: &str, kind: &str, out: &mut Vec<Token>) {
    let kind = group
        .get("$type")
        .and_then(Value::as_str)
        .unwrap_or(kind)
        .to_string();
    for (key, value) in group {
        if key.starts_with('$') {
            continue;
        }
        let Some(object) = value.as_object() else {
            // A bare value: a plain `{"primary": "#fff"}` reads as a token too.
            if let Some(text) = value_text(value) {
                out.push(Token {
                    path: joined(prefix, key),
                    kind: kind.clone(),
                    value: text,
                });
            }
            continue;
        };
        if let Some(value) = object.get("$value") {
            if let Some(text) = value_text(value) {
                out.push(Token {
                    path: joined(prefix, key),
                    kind: object
                        .get("$type")
                        .and_then(Value::as_str)
                        .unwrap_or(&kind)
                        .to_string(),
                    value: text,
                });
            }
        } else {
            walk(object, &joined(prefix, key), &kind, out);
        }
    }
}

fn joined(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// A token's value as CSS text: a string as it is, a number as it is, a
/// list joined with commas (a font stack), a shadow object composed.
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(value_text)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Value::Object(shadow) => {
            let part = |key: &str| shadow.get(key).and_then(value_text).unwrap_or_default();
            let (x, y, blur, color) = (
                part("offsetX"),
                part("offsetY"),
                part("blur"),
                part("color"),
            );
            (!color.is_empty()).then(|| {
                let spread = part("spread");
                if spread.is_empty() {
                    format!("{x} {y} {blur} {color}")
                } else {
                    format!("{x} {y} {blur} {spread} {color}")
                }
            })
        }
        Value::Null => None,
    }
}

/// A value a CSS variable may hold: no braces, no semicolon, bounded.
fn is_safe_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 300
        && !value.contains(['{', '}', ';', '<', '>'])
        && !value.contains("/*")
}

/// The stylesheet derived from the tokens: `:root { --color-primary: …; }`,
/// every token a variable, in path order. A value that cannot sit in a
/// variable is left out.
#[must_use]
pub fn css_of(tokens: &[Token]) -> String {
    let mut css = String::from(
        "/* The design system's tokens as CSS variables — derived from tokens.json by the \
         harness, never edited by hand. */\n:root {\n",
    );
    for token in tokens.iter().filter(|t| is_safe_value(&t.value)) {
        let _ = writeln!(css, "  {}: {};", variable_of(&token.path), token.value);
    }
    css.push_str("}\n");
    css
}

/// The tokens file the harness starts from: every token of [`TOKENS`] at
/// its neutral fallback, in the W3C format.
#[must_use]
pub fn default_tokens_json() -> String {
    let mut root = serde_json::Map::new();
    for spec in &TOKENS {
        let (group, name) = spec.path.split_once('.').unwrap_or(("", spec.path));
        let entry = root
            .entry(group.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(object) = entry.as_object_mut() {
            object.insert(
                name.to_string(),
                serde_json::json!({ "$type": spec.kind, "$value": spec.fallback }),
            );
        }
    }
    serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default()
}

/// The tokens of [`TOKENS`] a tokens file leaves out.
#[must_use]
pub fn missing(tokens: &[Token]) -> Vec<&'static str> {
    TOKENS
        .iter()
        .filter(|spec| !tokens.iter().any(|t| t.path == spec.path))
        .map(|spec| spec.path)
        .collect()
}

/// A tokens document completed: every token of [`TOKENS`] the document
/// leaves out is added at its fallback, so a fragment never meets an unset
/// variable. Answers the document, pretty, and what was added.
///
/// # Errors
///
/// The text is not a JSON object.
pub fn completed(json: &str) -> Result<(String, Vec<&'static str>), String> {
    let found = tokens_of(json)?;
    let added = missing(&found);
    let mut root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    for spec in TOKENS.iter().filter(|s| added.contains(&s.path)) {
        let (group, name) = spec.path.split_once('.').unwrap_or(("", spec.path));
        let Some(object) = root.as_object_mut() else {
            break;
        };
        let entry = object
            .entry(group.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(group) = entry.as_object_mut() {
            group.insert(
                name.to_string(),
                serde_json::json!({ "$type": spec.kind, "$value": spec.fallback }),
            );
        }
    }
    Ok((
        serde_json::to_string_pretty(&root).unwrap_or_default(),
        added,
    ))
}

/// Where the design system stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignState {
    /// No tokens yet.
    #[default]
    None,
    /// Tokens injected or generated, not validated: nothing is generated
    /// from them yet.
    Candidate,
    /// Validated by the human: the kit may be generated, the code stage
    /// installs it.
    Validated,
}

/// Where the tokens came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Nothing yet.
    #[default]
    None,
    /// A tokens file the human handed over.
    Injected,
    /// Drawn by an AI from the moodboard and the logo.
    Generated,
}

/// `mock/design/design.json`: the design system's record (contract O).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Design {
    /// Always 1.
    #[serde(default = "one")]
    pub version: u32,
    /// Where it stands.
    #[serde(default)]
    pub state: DesignState,
    /// Where the tokens came from.
    #[serde(default)]
    pub source: Source,
    /// What the human wrote for the generation: the brand, the mood.
    #[serde(default)]
    pub brief: String,
    /// The moodboard's images, relative to `mock/design/`.
    #[serde(default)]
    pub moodboard: Vec<String>,
    /// The logo, relative to `mock/design/`, or empty.
    #[serde(default)]
    pub logo: String,
    /// A candidate waits beside the validated tokens
    /// (`mock/design/candidate.json`): generated or injected again.
    #[serde(default)]
    pub candidate: bool,
    /// When it was validated, ISO-8601, or empty.
    #[serde(default, rename = "validatedAt")]
    pub validated_at: String,
    /// The roadmap issue that installs it in the product, once validated —
    /// its URL, or empty.
    #[serde(default)]
    pub issue: String,
}

const fn one() -> u32 {
    1
}

impl Default for Design {
    fn default() -> Self {
        Self {
            version: 1,
            state: DesignState::None,
            source: Source::None,
            brief: String::new(),
            moodboard: Vec::new(),
            logo: String::new(),
            candidate: false,
            validated_at: String::new(),
            issue: String::new(),
        }
    }
}

/// Where a kit component stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    /// Listed, no fragment yet.
    #[default]
    Pending,
    /// A generation is under way.
    Running,
    /// Its fragment is there.
    Generated,
    /// The last generation failed; `why` says what happened.
    Failed,
}

/// One component of the kit, as `mock/kit/kit.json` lists it (contract O).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    /// `Carousel`: a capital, then letters and digits.
    pub name: String,
    /// The base it is styled from (`carousel`), or empty for a custom one.
    #[serde(default)]
    pub base: String,
    /// What it is, for a human.
    #[serde(default)]
    pub title: String,
    /// What it shows and does, visually.
    #[serde(default)]
    pub what: String,
    /// The human's prompt for a custom component, or what they added for
    /// a base one.
    #[serde(default)]
    pub prompt: String,
    /// A reference image, relative to `mock/kit/`, or empty.
    #[serde(default)]
    pub reference: String,
    /// Where it stands.
    #[serde(default)]
    pub state: ComponentState,
    /// Why the last generation failed, or empty.
    #[serde(default)]
    pub why: String,
    /// The component this one was forked from, or empty.
    #[serde(
        default,
        rename = "variantOf",
        skip_serializing_if = "String::is_empty"
    )]
    pub variant_of: String,
    /// What the code stage changed when it recoded the fragment for real,
    /// one line, or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recoded: String,
    /// When the fragment was last written, ISO-8601, or empty.
    #[serde(default, rename = "generatedAt")]
    pub generated_at: String,
}

/// `mock/kit/kit.json`: the kit's record (contract O).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kit {
    /// Always 1.
    #[serde(default = "one")]
    pub version: u32,
    /// The bases the human ticked, by id.
    #[serde(default)]
    pub checked: Vec<String>,
    /// The components, base and custom.
    #[serde(default)]
    pub components: Vec<Component>,
}

impl Default for Kit {
    fn default() -> Self {
        Self {
            version: 1,
            checked: DEFAULT_CHECKED.iter().map(|s| (*s).to_string()).collect(),
            components: Vec::new(),
        }
    }
}

impl Kit {
    /// The kit a text holds; an empty text is an empty kit.
    ///
    /// # Errors
    ///
    /// The text is not a kit.
    pub fn parse(json: Option<&str>) -> Result<Self, String> {
        json.map_or_else(
            || Ok(Self::default()),
            |text| serde_json::from_str(text).map_err(|e| format!("kit.json: {e}")),
        )
    }

    /// The component named `name`.
    #[must_use]
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    /// The component named `name`, to change.
    pub fn component_mut(&mut self, name: &str) -> Option<&mut Component> {
        self.components.iter_mut().find(|c| c.name == name)
    }

    /// The fragment's path of a component, relative to `mock/`.
    #[must_use]
    pub fn fragment_path(name: &str) -> String {
        format!("kit/{name}.html")
    }

    /// Lists every ticked base the kit does not list yet, pending, and
    /// drops the pending base components no longer ticked. Answers the
    /// names added.
    pub fn settle(&mut self) -> Vec<String> {
        let mut added = Vec::new();
        for base in BASE
            .iter()
            .filter(|b| self.checked.iter().any(|c| c == b.id))
        {
            if self.component(base.name).is_none() {
                self.components.push(Component {
                    name: base.name.to_string(),
                    base: base.id.to_string(),
                    title: base.title.to_string(),
                    what: base.what.to_string(),
                    ..Component::default()
                });
                added.push(base.name.to_string());
            }
        }
        self.components.retain(|c| {
            c.base.is_empty()
                || c.state != ComponentState::Pending
                || self.checked.contains(&c.base)
        });
        added
    }
}

/// A component's name: a capital, then letters and digits, at most 60.
#[must_use]
pub fn is_name(name: &str) -> bool {
    (1..=60).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The component's name a human's title makes, unique among `taken`.
///
/// Its words, accents dropped, each capitalised and joined (`Portrait du
/// héros` → `PortraitDuHeros`), with a number added when `taken` already
/// has it — `None` when the title has no letter to make one from.
#[must_use]
pub fn name_for(title: &str, taken: &[&str]) -> Option<String> {
    let mut name = String::new();
    for word in title.split(|c: char| !c.is_alphanumeric()) {
        let plain: String = word.chars().flat_map(plain_letters).collect();
        let mut chars = plain.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(chars);
        }
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert_str(0, "Kit");
    }
    let stem: String = name.chars().take(56).collect();
    if !is_name(&stem) {
        return None;
    }
    let free = |candidate: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    if free(&stem) {
        return Some(stem);
    }
    (2..1000)
        .map(|n| format!("{stem}{n}"))
        .find(|candidate| free(candidate))
}

/// A character as ASCII letters and digits: an accented Latin letter as its
/// plain one, anything else dropped.
fn plain_letters(c: char) -> Vec<char> {
    if c.is_ascii_alphanumeric() {
        return vec![c];
    }
    let plain = match c.to_lowercase().next().unwrap_or(c) {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
        'ç' => "c",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ý' | 'ÿ' => "y",
        'œ' => "oe",
        'æ' => "ae",
        'ß' => "ss",
        _ => "",
    };
    let plain: Vec<char> = plain.chars().collect();
    if c.is_uppercase() {
        plain.iter().map(char::to_ascii_uppercase).collect()
    } else {
        plain
    }
}

/// One base of the kit: a neutral fragment the generation styles with the
/// product's tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base {
    /// `carousel`.
    pub id: &'static str,
    /// The component's name, `Carousel`.
    pub name: &'static str,
    /// For a human.
    pub title: &'static str,
    /// What it shows and does, visually.
    pub what: &'static str,
    /// The neutral fragment.
    pub fragment: &'static str,
}

/// The sixteen bases the harness ships.
pub const BASE: [Base; 16] = [
    Base {
        id: "button",
        name: "Button",
        title: "Button",
        what: "Primary, secondary and ghost buttons, with a disabled state.",
        fragment: include_str!("../assets/kit/button.html"),
    },
    Base {
        id: "input",
        name: "Input",
        title: "Text input",
        what: "A labelled text field with a hint, an error state and a disabled state.",
        fragment: include_str!("../assets/kit/input.html"),
    },
    Base {
        id: "select",
        name: "Select",
        title: "Drop list",
        what: "A labelled list to pick one option from, opened by a click.",
        fragment: include_str!("../assets/kit/select.html"),
    },
    Base {
        id: "checkbox",
        name: "Checkbox",
        title: "Checkbox",
        what: "A group of options to tick, several at once.",
        fragment: include_str!("../assets/kit/checkbox.html"),
    },
    Base {
        id: "radio",
        name: "Radio",
        title: "Radio group",
        what: "A group of options to pick one from.",
        fragment: include_str!("../assets/kit/radio.html"),
    },
    Base {
        id: "toggle",
        name: "Toggle",
        title: "Toggle switch",
        what: "An on/off switch with its label.",
        fragment: include_str!("../assets/kit/toggle.html"),
    },
    Base {
        id: "menu",
        name: "Menu",
        title: "Navigation menu",
        what: "A horizontal navigation bar with a brand, links and the current one marked.",
        fragment: include_str!("../assets/kit/menu.html"),
    },
    Base {
        id: "tabs",
        name: "Tabs",
        title: "Tabs",
        what: "A row of tabs that switch the panel shown below.",
        fragment: include_str!("../assets/kit/tabs.html"),
    },
    Base {
        id: "card",
        name: "Card",
        title: "Card",
        what: "A surface with a title, a body and actions.",
        fragment: include_str!("../assets/kit/card.html"),
    },
    Base {
        id: "modal",
        name: "Modal",
        title: "Modal dialog",
        what: "A dialog over the page, opened by a button, closed by its buttons or Escape.",
        fragment: include_str!("../assets/kit/modal.html"),
    },
    Base {
        id: "table",
        name: "Table",
        title: "Data table",
        what: "A table with a header row, striped rows and a numeric column.",
        fragment: include_str!("../assets/kit/table.html"),
    },
    Base {
        id: "carousel",
        name: "Carousel",
        title: "Carousel",
        what: "Slides that turn by arrows and dots, and on their own every few seconds.",
        fragment: include_str!("../assets/kit/carousel.html"),
    },
    Base {
        id: "badge",
        name: "Badge",
        title: "Badge",
        what: "Small labels: neutral, success, danger, accent.",
        fragment: include_str!("../assets/kit/badge.html"),
    },
    Base {
        id: "toast",
        name: "Toast",
        title: "Toast",
        what: "A short message that slides in at a corner and leaves on its own.",
        fragment: include_str!("../assets/kit/toast.html"),
    },
    Base {
        id: "pagination",
        name: "Pagination",
        title: "Pagination",
        what: "Previous, numbered pages and next, the current one marked.",
        fragment: include_str!("../assets/kit/pagination.html"),
    },
    Base {
        id: "breadcrumb",
        name: "Breadcrumb",
        title: "Breadcrumb",
        what: "The path to the current page, each step a link.",
        fragment: include_str!("../assets/kit/breadcrumb.html"),
    },
];

/// The bases ticked by default: the ones every application starts with.
pub const DEFAULT_CHECKED: [&str; 5] = ["button", "input", "card", "table", "menu"];

/// The base with this id.
#[must_use]
pub fn base(id: &str) -> Option<&'static Base> {
    BASE.iter().find(|b| b.id == id)
}

/// What a fragment must not do to be reused as it is: the sentence per
/// rule broken. Empty when it passes.
///
/// The rules: it weighs at most [`MAX_FRAGMENT`] characters; it reaches no
/// network (`fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`,
/// `sendBeacon`, a dynamic `import`); it loads no external script or
/// stylesheet; it carries no `data-kit` of its own (the loader would fill it
/// again, endlessly); it consumes the design system — at least one `var(--…)` —
/// and names no colour literal outside a variable's fallback.
#[must_use]
pub fn check(fragment: &str) -> Vec<String> {
    let mut found = Vec::new();
    if fragment.chars().count() > MAX_FRAGMENT {
        found.push(format!("weighs more than {MAX_FRAGMENT} characters"));
    }
    let lower = fragment.to_ascii_lowercase();
    for (needle, what) in [
        ("fetch(", "reaches the network with `fetch`"),
        (
            "xmlhttprequest",
            "reaches the network with `XMLHttpRequest`",
        ),
        ("websocket", "opens a `WebSocket`"),
        ("eventsource", "opens an `EventSource`"),
        ("sendbeacon", "sends a beacon"),
        ("import(", "loads a module at run time"),
        ("<script src", "loads an external script"),
        ("<link", "loads an external stylesheet"),
        ("@import", "imports an external stylesheet"),
        ("<iframe", "embeds a frame"),
    ] {
        if lower.contains(needle) {
            found.push(what.to_string());
        }
    }
    if lower.contains("data-kit=") {
        found.push(
            "carries a `data-kit` itself: the host element does, and the loader would fill it again"
                .to_string(),
        );
    }
    if !lower.contains("var(--") {
        found.push("consumes no token: no `var(--…)`".to_string());
    }
    let literals = colour_literals(&strip_fallbacks(fragment));
    if !literals.is_empty() {
        found.push(format!(
            "names a colour outside a variable's fallback: {}",
            literals.join(", ")
        ));
    }
    found
}

/// The text without the fallbacks of its `var(--x, …)` — what may hold a
/// literal colour.
fn strip_fallbacks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("var(") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        // Find the matching close paren.
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in from.char_indices() {
            match c {
                '(' => depth = depth.saturating_add(1),
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push_str(from);
            return out;
        };
        let call = from.get(..=end).unwrap_or_default();
        // Keep the variable's name, drop the fallback.
        let name_end = call
            .find(',')
            .unwrap_or_else(|| call.len().saturating_sub(1));
        out.push_str(call.get(..name_end).unwrap_or_default());
        out.push(')');
        rest = from.get(end.saturating_add(1)..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The colour literals of a text: `#rgb`, `#rrggbb`, `rgb(`, `rgba(`,
/// `hsl(`, `hsla(` — looked for in style blocks and style attributes only,
/// never in markup text or an SVG's own fill.
fn colour_literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for region in style_regions(text) {
        let lower = region.to_ascii_lowercase();
        let bytes = lower.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'#' {
                let hex: String = lower
                    .get(i.saturating_add(1)..)
                    .unwrap_or_default()
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .collect();
                if matches!(hex.len(), 3 | 4 | 6 | 8) {
                    let next = lower
                        .get(i.saturating_add(1).saturating_add(hex.len())..)
                        .and_then(|s| s.chars().next());
                    if !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                        found.push(format!("#{hex}"));
                    }
                }
                i = i.saturating_add(1).saturating_add(hex.len());
                continue;
            }
            for fun in ["rgba(", "rgb(", "hsla(", "hsl("] {
                if lower.get(i..).is_some_and(|s| s.starts_with(fun))
                    && !lower
                        .get(..i)
                        .and_then(|s| s.chars().last())
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    found.push(fun.trim_end_matches('(').to_string());
                }
            }
            i = i.saturating_add(1);
        }
    }
    found.sort();
    found.dedup();
    found.truncate(8);
    found
}

/// The style blocks and the `style="…"` attributes of a fragment.
fn style_regions(text: &str) -> Vec<&str> {
    let mut regions = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|s| s.find("<style")) {
        let start = from.saturating_add(at);
        let Some(open) = lower.get(start..).and_then(|s| s.find('>')) else {
            break;
        };
        let body_start = start.saturating_add(open).saturating_add(1);
        let end = lower
            .get(body_start..)
            .and_then(|s| s.find("</style"))
            .map_or(text.len(), |e| body_start.saturating_add(e));
        if let Some(region) = text.get(body_start..end) {
            regions.push(region);
        }
        from = end;
    }
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|s| s.find("style=\"")) {
        let start = from.saturating_add(at).saturating_add(7);
        let end = lower
            .get(start..)
            .and_then(|s| s.find('"'))
            .map_or(text.len(), |e| start.saturating_add(e));
        if let Some(region) = text.get(start..end) {
            regions.push(region);
        }
        from = end;
    }
    regions
}

/// Whether a page or a fragment uses the kit component `name`:
/// `data-kit="<name>"` somewhere in it.
#[must_use]
pub fn uses(html: &str, name: &str) -> bool {
    html.contains(&format!("data-kit=\"{name}\"")) || html.contains(&format!("data-kit='{name}'"))
}

/// The kit components a page or a fragment uses, by name, each once.
#[must_use]
pub fn used_in(html: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-kit=") {
        let after = rest.get(at.saturating_add(9)..).unwrap_or_default();
        let quote = after.chars().next();
        let name: String = after
            .get(usize::from(matches!(quote, Some('"' | '\'')))..)
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if is_name(&name) && !names.contains(&name) {
            names.push(name);
        }
        rest = after;
    }
    names
}

/// The most a skeleton weighs, in characters: a few lines of a prompt.
pub const MAX_SKELETON: usize = 1_200;

/// The elements that never close.
const VOID: [&str; 8] = ["input", "br", "img", "hr", "meta", "link", "source", "wbr"];

/// The attributes a skeleton keeps: what says which part is which.
const KEPT: [&str; 4] = ["class", "type", "role", "aria-checked"];

/// A fragment as a model needs it to reuse the component, and no more.
///
/// Its elements and their classes, the `<style>`, the `<script>` and the
/// comments gone, a sibling repeated kept once, every text an ellipsis —
/// `<table class="kit-table"><thead><tr><th>…</th></tr></thead>…`. What a
/// micro-frontend's generation reads of the kit instead of its code: the
/// look is `kit.css`'s, the micro-frontend writes the markup with its data.
#[must_use]
pub fn skeleton(fragment: &str) -> String {
    let text = without_blocks(fragment);
    let mut out = String::new();
    // Per depth, the signatures of the siblings already written.
    let mut seen: Vec<Vec<String>> = vec![Vec::new()];
    let mut skipping: Option<usize> = None;
    let mut depth = 0usize;
    let mut texted = false;
    let mut rest = text.as_str();
    while let Some(lt) = rest.find('<') {
        let words = rest.get(..lt).unwrap_or_default();
        if skipping.is_none() && !words.trim().is_empty() && !texted {
            out.push('…');
            texted = true;
        }
        let after = rest.get(lt.saturating_add(1)..).unwrap_or_default();
        let Some(gt) = after.find('>') else {
            break;
        };
        let tag = after.get(..gt).unwrap_or_default().trim();
        rest = after.get(gt.saturating_add(1)..).unwrap_or_default();
        if let Some(closing) = tag.strip_prefix('/') {
            depth = depth.saturating_sub(1);
            seen.truncate(depth.saturating_add(1));
            if skipping == Some(depth) {
                skipping = None;
            } else if skipping.is_none() {
                let _ = write!(out, "</{}>", closing.trim().to_ascii_lowercase());
                texted = false;
            }
            continue;
        }
        let (name, attributes) = tag_of(tag);
        if name.is_empty() || name.starts_with('!') {
            continue;
        }
        let void = VOID.contains(&name.as_str()) || tag.ends_with('/');
        if skipping.is_none() {
            let class = attributes
                .iter()
                .find(|(k, _)| k == "class")
                .map(|(_, v)| v.as_str())
                .unwrap_or_default();
            let signature = format!("{name}.{class}");
            let siblings = seen.last_mut();
            if siblings.as_ref().is_some_and(|s| s.contains(&signature)) {
                if !void {
                    skipping = Some(depth);
                }
            } else {
                if let Some(siblings) = siblings {
                    siblings.push(signature);
                }
                out.push('<');
                out.push_str(&name);
                for (key, value) in attributes
                    .iter()
                    .filter(|(k, _)| KEPT.contains(&k.as_str()))
                {
                    let _ = write!(out, " {key}=\"{value}\"");
                }
                out.push('>');
                texted = false;
            }
        }
        if !void {
            depth = depth.saturating_add(1);
            seen.push(Vec::new());
        }
    }
    if out.chars().count() > MAX_SKELETON {
        out = out.chars().take(MAX_SKELETON).collect();
        out.push('…');
    }
    out
}

/// A fragment without its `<style>`, `<script>` and comments.
fn without_blocks(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    loop {
        let next = [
            ("<style", "</style>"),
            ("<script", "</script>"),
            ("<!--", "-->"),
        ]
        .iter()
        .filter_map(|(open, close)| rest.find(open).map(|at| (at, *close)))
        .min_by_key(|(at, _)| *at);
        let Some((at, close)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(rest.get(..at).unwrap_or_default());
        let after = rest.get(at..).unwrap_or_default();
        rest = after
            .find(close)
            .and_then(|end| after.get(end.saturating_add(close.len())..))
            .unwrap_or_default();
    }
    out
}

/// An opening tag's name, lower case, and its attributes in order.
fn tag_of(tag: &str) -> (String, Vec<(String, String)>) {
    let tag = tag.trim_end_matches('/').trim();
    let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let name = tag.get(..name_end).unwrap_or_default().to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut rest = tag.get(name_end..).unwrap_or_default().trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let key = rest.get(..key_end).unwrap_or_default().to_ascii_lowercase();
        rest = rest.get(key_end..).unwrap_or_default().trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'');
            if let Some(q) = quote {
                let inner = after.get(1..).unwrap_or_default();
                let end = inner.find(q).unwrap_or(inner.len());
                value = inner.get(..end).unwrap_or_default().to_string();
                rest = inner.get(end.saturating_add(1)..).unwrap_or_default();
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                value = after.get(..end).unwrap_or_default().to_string();
                rest = after.get(end..).unwrap_or_default();
            }
        }
        if !key.is_empty() {
            attributes.push((key, value));
        }
        rest = rest.trim_start();
    }
    (name, attributes)
}

/// One component as a line of the kit's index: its name, what it is, its
/// root class and the variants its markup shows —
/// `- Button (Button): primary, secondary… — kit-button: primary, secondary, ghost`.
#[must_use]
pub fn index_line(component: &Component, fragment: &str) -> String {
    let base = base(&component.base);
    let title = if component.title.is_empty() {
        base.map_or("custom", |b| b.title)
    } else {
        &component.title
    };
    let what = if component.what.is_empty() {
        base.map_or(component.prompt.as_str(), |b| b.what)
    } else {
        &component.what
    };
    let mut classes: Vec<String> = Vec::new();
    let shape = skeleton(fragment);
    let mut rest = shape.as_str();
    while let Some(at) = rest.find("class=\"") {
        let after = rest.get(at.saturating_add(7)..).unwrap_or_default();
        let end = after.find('"').unwrap_or(after.len());
        for class in after.get(..end).unwrap_or_default().split_whitespace() {
            if !classes.iter().any(|c| c == class) && classes.len() < 12 {
                classes.push(class.to_string());
            }
        }
        rest = after.get(end..).unwrap_or_default();
    }
    let what: String = what.chars().take(140).collect();
    match classes.split_first() {
        Some((root, variants)) if !variants.is_empty() => format!(
            "- {} ({title}): {what} — {root}: {}",
            component.name,
            variants.join(", ")
        ),
        Some((root, _)) => format!("- {} ({title}): {what} — {root}", component.name),
        None => format!("- {} ({title}): {what}", component.name),
    }
}

/// The kit's stylesheet: the `<style>` of every fragment given, as
/// `(name, fragment)`, in that order, under a header saying it is derived.
///
/// What the real frontend loads once, beside `tokens.css`: a fragment's
/// style is scoped under its root's class (`.kit-table`), so a Micro-UI
/// writes the kit's classes with its own data and wears the kit's look; a
/// component generated again changes this file, never the Micro-UI.
#[must_use]
pub fn stylesheet(fragments: &[(String, String)]) -> String {
    let mut css = String::from(
        "/* The kit's stylesheet: every mock/kit/<Name>.html's style, derived by the harness \
         on publish. Never edited by hand. */\n",
    );
    for (name, fragment) in fragments {
        let styles = styles_of(fragment);
        if styles.is_empty() {
            continue;
        }
        let _ = writeln!(css, "\n/* {name} */");
        for style in styles {
            css.push_str(style.trim());
            css.push('\n');
        }
    }
    css
}

/// The bodies of a fragment's `<style>` elements, in order.
fn styles_of(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<style") {
        let Some(after) = rest.get(open..) else {
            break;
        };
        let Some(body) = after
            .find('>')
            .and_then(|gt| after.get(gt.saturating_add(1)..))
        else {
            break;
        };
        let Some(close) = body.find("</style>") else {
            break;
        };
        found.push(body.get(..close).unwrap_or_default());
        rest = body
            .get(close.saturating_add("</style>".len())..)
            .unwrap_or_default();
    }
    found
}

/// The design system's showcase: a simulated page made of the kit only.
///
/// A "contact us" page — a menu, a form (fields, a drop list, radios,
/// checkboxes, switches, its buttons), a card with an image, a table of the
/// other ways to reach the company.
///
/// Coded in the harness like the bases: each component is filled by the
/// loader from its fragment, then its texts are fitted to the page, so the
/// page follows every component the canvas saves. Served beside the
/// previews, one folder down (`../mock.js`, `../design/tokens.css`).
pub const SHOWCASE: &str = include_str!("../assets/showcase.html");

/// The components the showcase is made of, by name.
#[must_use]
pub fn showcase_uses() -> Vec<String> {
    used_in(SHOWCASE)
}

/// The preview page of one kit component alone: the tokens, one `data-kit`
/// element, the loader.
///
/// Served beside the pages. No mock-up style: a fragment styles itself, and
/// the design system's branch has none.
#[must_use]
pub fn preview_page(name: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{name}</title>\
         <link rel=\"stylesheet\" href=\"../design/tokens.css\"></head>\n\
         <body style=\"margin:0;padding:16px;background:transparent\">\n\
         <div data-kit=\"{name}\"></div>\n\
         <script src=\"../mock.js\"></script>\n</body></html>\n"
    )
}

/// The tokens as a summary a session reads: `--color-primary: #… (color)`,
/// one per line.
#[must_use]
pub fn summary_of(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| format!("{}: {}", variable_of(&t.path), t.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tokens by group, for a sheet: `color` → its tokens…
#[must_use]
pub fn grouped(tokens: &[Token]) -> BTreeMap<String, Vec<Token>> {
    let mut groups: BTreeMap<String, Vec<Token>> = BTreeMap::new();
    for token in tokens {
        let group = token.path.split('.').next().unwrap_or_default().to_string();
        groups.entry(group).or_default().push(token.clone());
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_default_tokens_cover_every_spec_and_derive_into_variables() {
        let json = default_tokens_json();
        let tokens = tokens_of(&json).expect("the default is a tokens file");
        assert!(missing(&tokens).is_empty(), "expected nothing");
        let css = css_of(&tokens);
        assert!(css.contains("--color-primary: #2b6cb0;"));
        assert!(css.contains("--font-family: system-ui, sans-serif;"));
        assert!(css.contains("--shadow-low: 0 1px 3px rgba(0, 0, 0, 0.12);"));
        assert!(css.starts_with("/*"));
        assert!(css.trim_end().ends_with('}'));
    }

    #[test]
    fn a_tokens_file_is_read_whatever_its_shape() {
        let tokens = tokens_of(
            r##"{"color":{"$type":"color","primary":{"$value":"#123456"},"brand":{"deep":{"$value":"#000"}}},
                "font":{"family":{"$type":"fontFamily","$value":["Inter","sans-serif"]}},
                "shadow":{"low":{"$type":"shadow","$value":{"offsetX":"0","offsetY":"1px","blur":"2px","color":"#0003"}}},
                "flat":"#fff"}"##,
        )
        .expect("read");
        let get = |p: &str| tokens.iter().find(|t| t.path == p).map(|t| t.value.clone());
        assert_eq!(get("color.primary").as_deref(), Some("#123456"));
        assert_eq!(get("color.brand.deep").as_deref(), Some("#000"));
        assert_eq!(get("font.family").as_deref(), Some("Inter, sans-serif"));
        assert_eq!(get("shadow.low").as_deref(), Some("0 1px 2px #0003"));
        assert_eq!(get("flat").as_deref(), Some("#fff"));
        assert_eq!(
            tokens
                .iter()
                .find(|t| t.path == "color.primary")
                .map(|t| t.kind.as_str()),
            Some("color")
        );
        assert!(tokens_of("[]").is_err());
        assert!(tokens_of("nope").is_err());
        let (done, added) =
            completed(r##"{"color":{"primary":{"$value":"#123456"}}}"##).expect("ok");
        assert_eq!(added.len(), TOKENS.len() - 1);
        assert!(done.contains("#123456"));
        assert!(
            missing(&tokens_of(&done).expect("ok")).is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn a_value_that_cannot_sit_in_a_variable_is_left_out() {
        let tokens = vec![
            Token {
                path: "color.primary".into(),
                kind: "color".into(),
                value: "#fff; } body { display: none".into(),
            },
            Token {
                path: "space.1".into(),
                kind: "dimension".into(),
                value: "4px".into(),
            },
        ];
        let css = css_of(&tokens);
        assert!(!css.contains("display"));
        assert!(css.contains("--space-1: 4px;"));
    }

    #[test]
    fn every_base_fragment_passes_the_check_and_consumes_the_tokens() {
        for base in &BASE {
            let found = check(base.fragment);
            assert!(found.is_empty(), "{}: {found:?}", base.id);
            assert!(is_name(base.name));
            assert!(base.fragment.contains("data-kit") || base.fragment.contains("kit-"));
        }
        for id in DEFAULT_CHECKED {
            assert!(base(id).is_some(), "{id} is not a base");
        }
    }

    #[test]
    fn the_check_names_what_a_fragment_must_not_do() {
        let bad = "<div style=\"color:#fff\"><style>.x{background:rgb(1,2,3);color:var(--color-text,#111)}</style><script>fetch('/x');import('./y.js')</script><link rel=stylesheet href=x></div>";
        let found = check(bad);
        assert!(found.iter().any(|f| f.contains("fetch")));
        assert!(found.iter().any(|f| f.contains("module")));
        assert!(found.iter().any(|f| f.contains("stylesheet")));
        let colours = found
            .iter()
            .find(|f| f.contains("colour"))
            .expect("a colour literal");
        assert!(colours.contains("#fff") && colours.contains("rgb"));
        assert!(!colours.contains("#111"), "a fallback is allowed");
        assert!(check("<p>hi</p>").iter().any(|f| f.contains("no `var(--")));
        assert!(
            check("<p style=\"color:var(--color-text, #123)\">hi</p>").is_empty(),
            "expected nothing"
        );
        // A colour in markup text or an SVG fill is not a style.
        assert!(
            check("<svg fill=\"#abc\"><p style=\"color:var(--color-text)\">#fff</p></svg>")
                .is_empty(),
            "expected nothing"
        );
        // A fragment naming itself is filled again by the loader, endlessly.
        assert!(
            check(
                "<div class=\"kit-hero\" data-kit=\"hero\" style=\"color:var(--color-text)\"></div>"
            )
            .iter()
            .any(|f| f.contains("data-kit"))
        );
    }

    #[test]
    fn a_kit_settles_its_ticked_bases_and_finds_who_uses_a_component() {
        let mut kit = Kit::default();
        let added = kit.settle();
        assert_eq!(added.len(), DEFAULT_CHECKED.len());
        assert!(kit.component("Button").is_some());
        assert!(kit.component("Carousel").is_none());
        kit.checked.retain(|c| c != "button");
        kit.settle();
        assert!(
            kit.component("Button").is_none(),
            "a pending base unticked goes"
        );
        kit.checked.push("carousel".into());
        kit.component_mut("Card").expect("card").state = ComponentState::Generated;
        kit.checked.retain(|c| c != "card");
        assert_eq!(kit.settle(), ["Carousel"]);
        assert!(kit.component("Card").is_some(), "a generated one stays");
        assert!(Kit::parse(Some("nope")).is_err());
        assert_eq!(
            Kit::parse(None).expect("empty").checked.len(),
            DEFAULT_CHECKED.len()
        );
        assert_eq!(Kit::fragment_path("Carousel"), "kit/Carousel.html");
        let page = "<main><div data-kit=\"Carousel\"></div><div data-kit='Menu' data-kit=\"Carousel\"></div></main>";
        assert!(uses(page, "Carousel") && uses(page, "Menu") && !uses(page, "Card"));
        assert_eq!(used_in(page), ["Carousel", "Menu"]);
        assert!(preview_page("Carousel").contains("data-kit=\"Carousel\""));
        // The showcase is made of bases only, each a default the human
        // starts with, and shows every one of them.
        let uses = showcase_uses();
        assert_eq!(uses.len(), DEFAULT_CHECKED.len(), "{uses:?}");
        for name in &uses {
            let base = BASE.iter().find(|b| b.name == name).expect("a base");
            assert!(
                DEFAULT_CHECKED.contains(&base.id),
                "{name} ticked by default"
            );
        }
        assert!(SHOWCASE.contains("../mock.js") && SHOWCASE.contains("../design/tokens.css"));
        assert!(!preview_page("Carousel").contains("style.css"));
        let json = serde_json::to_string(&kit).expect("json");
        assert!(!json.contains("variantOf"), "an empty fork is not written");
        assert_eq!(Kit::parse(Some(&json)).expect("round trip"), kit);
        let design = Design::default();
        let text = serde_json::to_string(&design).expect("json");
        assert_eq!(serde_json::from_str::<Design>(&text).expect("back"), design);
        assert_eq!(
            summary_of(&[Token {
                path: "color.primary".into(),
                kind: "color".into(),
                value: "#1".into()
            }]),
            "--color-primary: #1"
        );
        assert_eq!(
            grouped(&tokens_of(&default_tokens_json()).expect("ok")).len(),
            6
        );
    }

    #[test]
    fn a_title_names_a_component_and_a_fragment_is_read_without_its_code() {
        // A human's title names the component.
        assert_eq!(name_for("Hero frame", &[]).as_deref(), Some("HeroFrame"));
        assert_eq!(
            name_for("Portrait du héros", &[]).as_deref(),
            Some("PortraitDuHeros")
        );
        assert_eq!(
            name_for("bouton d'état", &[]).as_deref(),
            Some("BoutonDEtat")
        );
        assert_eq!(name_for("3 colonnes", &[]).as_deref(), Some("Kit3Colonnes"));
        assert_eq!(
            name_for("hero frame", &["HeroFrame", "heroframe2"]).as_deref(),
            Some("HeroFrame3")
        );
        assert_eq!(name_for("!!! ???", &[]), None);
        // A skeleton: the markup's shape, its classes, a repeated sibling once, no style, no script, no text.
        let table = base("table").expect("table").fragment;
        let shape = skeleton(table);
        assert_eq!(
            shape,
            "<table class=\"kit-table\"><thead><tr><th>…</th><th class=\"num\">…</th></tr></thead><tbody><tr><td>…</td><td class=\"num\">…</td></tr></tbody></table>"
        );
        assert_eq!(
            shape.matches("<tr>").count(),
            2,
            "one header row, one body row: {shape}"
        );
        assert!(
            !shape.contains("<style") && !shape.contains("Jane") && !shape.contains("kit-table {")
        );
        assert!(shape.len() < table.len() / 3);
        let button = base("button").expect("button");
        let line = index_line(
            &Component {
                name: "Button".into(),
                base: "button".into(),
                ..Component::default()
            },
            button.fragment,
        );
        assert!(line.starts_with("- Button (Button): "), "{line}");
        assert!(
            line.contains("kit-button: primary, secondary, ghost, danger"),
            "{line}"
        );
        assert_eq!(
            skeleton("<p>a<br>b</p><!-- x --><input type=\"text\" class=\"f\"/>"),
            "<p>…<br>…</p><input type=\"text\" class=\"f\">"
        );
        // The kit's stylesheet: each fragment's style, the markup and the scripts left out.
        let css = stylesheet(&[
            (
                "Table".to_string(),
                base("table").expect("table").fragment.to_string(),
            ),
            ("Plain".to_string(), "<p>no style</p>".to_string()),
            (
                "Two".to_string(),
                "<style>.a{}</style><b></b><style media=\"x\">.b{}</style>".to_string(),
            ),
        ]);
        assert!(css.contains("/* Table */") && css.contains(".kit-table"));
        assert!(!css.contains("/* Plain */") && !css.contains("<table") && !css.contains("<style"));
        assert!(css.contains(".a{}\n.b{}"));
    }

    proptest! {
        #[test]
        fn stripping_fallbacks_never_panics_and_keeps_the_names(text in "\\PC{0,200}") {
            let out = strip_fallbacks(&text);
            prop_assert!(out.len() <= text.len() + 1);
            let _ = colour_literals(&out);
            let _ = used_in(&text);
        }

        #[test]
        fn the_stylesheet_never_panics_and_keeps_its_header(text in "(\\PC|<style>|</style>){0,60}") {
            let css = stylesheet(&[("X".to_string(), text)]);
            prop_assert!(css.starts_with("/* The kit's stylesheet"));
        }

        #[test]
        fn a_title_makes_a_valid_name_or_none(title in "\\PC{0,120}") {
            if let Some(name) = name_for(&title, &["Button"]) {
                prop_assert!(is_name(&name), "{name}");
            }
        }

        #[test]
        fn a_skeleton_never_panics_and_stays_light(text in "(\\PC|<[a-z/!]{0,6}|>| class=\"[a-z ]{0,8}\"){0,120}") {
            prop_assert!(skeleton(&text).chars().count() <= MAX_SKELETON + 1);
        }

        #[test]
        fn a_name_is_a_capital_then_alphanumerics(name in "[A-Z][A-Za-z0-9]{0,59}") {
            prop_assert!(is_name(&name));
            prop_assert!(!is_name(&name.to_lowercase()));
        }
    }
}

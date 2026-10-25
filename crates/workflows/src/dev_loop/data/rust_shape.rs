//! The public shape of one Rust file: items, fields, variants, signatures,
//! no bodies — the scan [`signatures`](super::signatures) carries.
//!
//! Deliberately a scan and not a parse, with no dependency. It reads the
//! source once with what cannot hold a signature taken out — string and
//! character literals emptied, comments dropped, doc comments set apart —
//! so a brace in a string never miscounts, then walks the items:
//!
//! - a signature written over several lines is joined into one, up to the
//!   `{` or `;` that ends it at bracket depth 0;
//! - a `trait` keeps every method's signature, a defaulted one marked
//!   `{ … }` (what an implementer may leave out), a required one `;`;
//! - an `impl` keeps its header and its public methods;
//! - a `struct` keeps its public fields and counts the others, an `enum`
//!   keeps its variants;
//! - `pub const`, `pub static`, `pub type`, `pub use` and `pub mod` are kept,
//!   a long constant's value cut;
//! - a `#[cfg(test)]` item and every private item are skipped, bodies and all.
//!
//! [`Visibility::External`] is for a crate the product depends on: only
//! `pub` is API there (`pub(crate)` is not), and a function, a trait and a
//! trait's method keep the first line of their doc comment, which says what
//! they are for — a type's fields and an impl's methods go without, to fit
//! the engine's five modules a unit uses in its budget.
//!
//! What it can get wrong is a slightly wrong index, never a panic and never
//! an unbalanced signature: an item whose brackets do not close is dropped.

/// Which items are the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// The checkout's own code: `pub(crate)` and `pub(super)` items are
    /// what a neighbour module calls, and are kept.
    Crate,
    /// A dependency's code: only `pub` is reachable, and a function, a trait
    /// and its methods keep the first line of their doc comment.
    External,
}

/// One line of the cleaned source.
struct Line {
    /// The code, literals emptied and comments dropped.
    code: Vec<char>,
    /// The text of a doc comment (`///`), when the line is one.
    doc: Option<String>,
}

/// Where the reading stands: in code, in a string (raw: its `#` count), in
/// a block comment (its nesting).
#[derive(Clone, Copy)]
enum Lexing {
    Code,
    Text(Option<usize>),
    Comment(usize),
}

const fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The source, cleaned: see the module.
fn clean(source: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut state = Lexing::Code;
    for raw in source.lines() {
        let trimmed = raw.trim_start();
        if matches!(state, Lexing::Code) {
            if let Some(doc) = trimmed.strip_prefix("///")
                && !doc.starts_with('/')
            {
                out.push(Line {
                    code: Vec::new(),
                    doc: Some(doc.trim().to_string()),
                });
                continue;
            }
            if trimmed.starts_with("//!") {
                out.push(Line {
                    code: Vec::new(),
                    doc: None,
                });
                continue;
            }
        }
        let chars: Vec<char> = raw.chars().collect();
        let mut code = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let Some((next, then)) = lex(&chars, i, state, &mut code) else {
                break;
            };
            i = next;
            state = then;
        }
        out.push(Line { code, doc: None });
    }
    out
}

/// One step of the reading at `chars[i]` in `state`: what it keeps in
/// `code`, where the next step starts and in which state. `None` when the
/// rest of the line is a comment.
fn lex(chars: &[char], i: usize, state: Lexing, code: &mut Vec<char>) -> Option<(usize, Lexing)> {
    let at = |n: usize| chars.get(n).copied();
    let c = chars[i];
    let step = match state {
        Lexing::Comment(depth) if c == '*' && at(i + 1) == Some('/') => (
            i + 2,
            if depth <= 1 {
                Lexing::Code
            } else {
                Lexing::Comment(depth - 1)
            },
        ),
        Lexing::Comment(depth) if c == '/' && at(i + 1) == Some('*') => {
            (i + 2, Lexing::Comment(depth + 1))
        }
        Lexing::Text(None) if c == '\\' => (i + 2, state),
        Lexing::Text(None) if c == '"' => {
            code.push('"');
            (i + 1, Lexing::Code)
        }
        Lexing::Text(Some(hashes)) if c == '"' && (1..=hashes).all(|n| at(i + n) == Some('#')) => {
            code.push('"');
            (i + 1 + hashes, Lexing::Code)
        }
        Lexing::Comment(_) | Lexing::Text(_) => (i + 1, state),
        Lexing::Code => return lex_code(chars, i, code),
    };
    Some(step)
}

/// [`lex`] in code: a comment starts, a literal is emptied, or the
/// character is kept.
fn lex_code(chars: &[char], i: usize, code: &mut Vec<char>) -> Option<(usize, Lexing)> {
    let at = |n: usize| chars.get(n).copied();
    let c = chars[i];
    let after_ident = i > 0 && ident(chars[i - 1]);
    let raw_from = match c {
        'r' => Some(i + 1),
        'b' if at(i + 1) == Some('r') => Some(i + 2),
        _ => None,
    }
    .filter(|_| !after_ident);
    if c == '/' && at(i + 1) == Some('/') {
        return None;
    }
    let step = if c == '/' && at(i + 1) == Some('*') {
        (i + 2, Lexing::Comment(1))
    } else if c == '"' || (!after_ident && c == 'b' && at(i + 1) == Some('"')) {
        code.extend(['"', '…']);
        (if c == '"' { i + 1 } else { i + 2 }, Lexing::Text(None))
    } else if let Some((from, hashes)) =
        raw_from.and_then(|from| raw_string(chars, from).map(|hashes| (from, hashes)))
    {
        code.extend(['"', '…']);
        (from + hashes + 1, Lexing::Text(Some(hashes)))
    } else if c == '\'' && at(i + 1) == Some('\\') {
        let mut end = i + 3;
        while end < chars.len() && chars[end] != '\'' {
            end += 1;
        }
        code.extend(['\'', '…', '\'']);
        (end + 1, Lexing::Code)
    } else if c == '\'' && at(i + 2) == Some('\'') {
        code.extend(['\'', '…', '\'']);
        (i + 3, Lexing::Code)
    } else {
        code.push(c);
        (i + 1, Lexing::Code)
    };
    Some(step)
}

/// The `#` count of a raw string whose `#`s start at `from`, when a `"`
/// follows them.
fn raw_string(chars: &[char], from: usize) -> Option<usize> {
    let hashes = chars.get(from..)?.iter().take_while(|c| **c == '#').count();
    (chars.get(from + hashes) == Some(&'"')).then_some(hashes)
}

/// A place in the cleaned source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct At {
    line: usize,
    col: usize,
}

/// What ends the text [`join`] reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Until {
    /// A `{` (a body or a block of members) or a `;`, at depth 0.
    BodyOrEnd,
    /// A `;` at depth 0, braces counted: a constant, a `use` list.
    End,
    /// A `,` at depth 0, or the `}` that closes the block: a field, a variant.
    Member,
}

/// Text read across lines up to its terminator.
struct Joined {
    text: String,
    end: char,
    after: At,
}

/// The nesting [`join`] counts: brackets, braces, and the angles of
/// generics — `<`/`>` read as such only outside brackets and braces and
/// before a value's `=`, where they compare or shift (`[u8; 1 << 4]`).
#[derive(Default)]
struct Depth {
    brackets: i32,
    braces: i32,
    angles: i32,
    /// A top-level `=` was read: what follows is a value, where `<` and `>`
    /// compare or shift.
    valued: bool,
}

impl Depth {
    /// Every count at 0.
    const fn level(&self) -> bool {
        self.brackets == 0 && self.braces == 0 && self.angles == 0
    }

    /// `c`, between `before` and `after`, counted. `false` when a closing
    /// mark has nothing open: the text does not balance.
    const fn count(&mut self, before: Option<char>, c: char, after: Option<char>) -> bool {
        let generics = !self.valued && self.braces == 0 && self.brackets == 0;
        match c {
            '(' | '[' => self.brackets += 1,
            ')' | ']' => self.brackets -= 1,
            '{' => self.braces += 1,
            '}' => self.braces -= 1,
            '<' if generics => self.angles += 1,
            // The `>` of `->` and `=>` closes nothing.
            '>' if generics && !matches!(before, Some('-' | '=')) => self.angles -= 1,
            '=' if self.level()
                && !matches!(
                    before,
                    Some('=' | '!' | '<' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^')
                )
                && !matches!(after, Some('=' | '>')) =>
            {
                self.valued = true;
            }
            _ => {}
        }
        self.brackets >= 0 && self.braces >= 0 && self.angles >= 0
    }
}

/// Reads from `from` up to what `until` names, the lines joined into one:
/// `None` when the brackets do not balance or the file ends first. A `,`
/// inside `BTreeMap<K, V>` ends nothing: generics count as brackets.
fn join(lines: &[Line], from: At, until: Until) -> Option<Joined> {
    let mut pieces: Vec<String> = Vec::new();
    let mut depth = Depth::default();
    let mut at = from;
    while at.line < lines.len() {
        let code = &lines[at.line].code;
        let mut piece = String::new();
        while at.col < code.len() {
            let c = code[at.col];
            let before = at.col.checked_sub(1).and_then(|i| code.get(i)).copied();
            let after = code.get(at.col + 1).copied();
            at.col += 1;
            let end = depth.level()
                && match until {
                    Until::BodyOrEnd => matches!(c, '{' | ';'),
                    Until::End => c == ';',
                    Until::Member => matches!(c, ',' | '}'),
                };
            if end {
                pieces.push(piece);
                return Some(Joined {
                    text: tidy(&pieces),
                    end: c,
                    after: at,
                });
            }
            if !depth.count(before, c, after) {
                return None;
            }
            piece.push(c);
        }
        pieces.push(piece);
        at = At {
            line: at.line + 1,
            col: 0,
        };
    }
    None
}

/// Pieces of one signature, one per line, as one line of text.
fn tidy(pieces: &[String]) -> String {
    let mut out = String::new();
    for piece in pieces.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let glued = out.ends_with(['(', '[', '<'])
            || piece.starts_with([')', ']', '>', ',', '.', '?'])
            || out.is_empty();
        if !glued {
            out.push(' ');
        }
        out.push_str(piece);
    }
    let mut text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        (",)", ")"),
        (", )", ")"),
        (",]", "]"),
        (",>", ">"),
        (", >", ">"),
        (", }", " }"),
    ] {
        text = text.replace(from, to);
    }
    text
}

/// The place after the `}` that closes the block whose `{` was just read.
fn past_block(lines: &[Line], from: At) -> At {
    let mut depth = 1_i32;
    let mut at = from;
    while at.line < lines.len() {
        let code = &lines[at.line].code;
        while at.col < code.len() {
            let c = code[at.col];
            at.col += 1;
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return at;
                }
            }
        }
        at = At {
            line: at.line + 1,
            col: 0,
        };
    }
    at
}

/// The first place at or after `from` that holds code, docs counted as
/// their own lines.
fn next_code(lines: &[Line], from: At) -> At {
    let mut at = from;
    while at.line < lines.len() {
        let line = &lines[at.line];
        if line.doc.is_some() {
            return at;
        }
        while at.col < line.code.len() && line.code[at.col].is_whitespace() {
            at.col += 1;
        }
        if at.col < line.code.len() {
            return at;
        }
        at = At {
            line: at.line + 1,
            col: 0,
        };
    }
    at
}

/// The rest of the line from `at`, as text.
fn rest(lines: &[Line], at: At) -> String {
    lines
        .get(at.line)
        .map(|line| line.code.get(at.col..).unwrap_or_default().iter().collect())
        .unwrap_or_default()
}

/// The item's first lines from `at`, joined: what [`head`] reads, so a
/// `pub` alone on its line still says what follows it.
fn peek(lines: &[Line], at: At) -> String {
    let mut text = rest(lines, at);
    for line in lines.iter().skip(at.line + 1).take(3) {
        text.push(' ');
        text.extend(line.code.iter());
    }
    text
}

/// What an item is, read off its first words.
struct Head {
    /// Visible as the shape asks.
    kept: bool,
    /// `struct`, `enum`, `union`, `trait`, `fn`, `type`, `const`, `static`,
    /// `mod`, `use`, `impl`, or empty for anything else.
    kind: &'static str,
}

fn head(text: &str, visibility: Visibility) -> Head {
    let mut rest = text.trim_start();
    let mut public = false;
    let mut restricted = false;
    if let Some(after) = rest.strip_prefix("pub(") {
        restricted = true;
        rest = after
            .split_once(')')
            .map_or("", |(_, tail)| tail)
            .trim_start();
    } else if let Some(after) = rest.strip_prefix("pub ") {
        public = true;
        rest = after.trim_start();
    }
    loop {
        let before = rest;
        for qualifier in ["async ", "unsafe ", "default ", "extern \"…\" ", "extern "] {
            rest = rest.strip_prefix(qualifier).unwrap_or(rest).trim_start();
        }
        if let Some(after) = rest.strip_prefix("const ")
            && ["fn ", "unsafe ", "async ", "extern "]
                .iter()
                .any(|next| after.trim_start().starts_with(next))
        {
            rest = after.trim_start();
        }
        if rest == before {
            break;
        }
    }
    let kinds = [
        "struct", "enum", "union", "trait", "fn", "type", "const", "static", "mod", "use", "impl",
    ];
    let kind = kinds
        .into_iter()
        .find(|kind| {
            rest.strip_prefix(kind).is_some_and(|after| {
                after.is_empty() || !ident(after.chars().next().unwrap_or(' '))
            })
        })
        .unwrap_or("");
    let kept = public || (restricted && visibility == Visibility::Crate);
    Head { kept, kind }
}

/// What holds the items being read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    File,
    Impl,
    Trait,
}

/// A constant's signature, its value cut when it is long.
fn constant(text: &str) -> String {
    const LONG: usize = 100;
    if text.chars().count() <= LONG {
        return text.to_string();
    }
    text.split_once(" = ")
        .map_or_else(|| text.to_string(), |(name, _)| format!("{name} = …"))
}

/// The item at `at` skipped whole: its body, or up to its `;`.
fn skip_item(lines: &[Line], at: At) -> At {
    match join(lines, at, Until::BodyOrEnd) {
        Some(joined) if joined.end == '{' => past_block(lines, joined.after),
        Some(joined) => joined.after,
        None => At {
            line: at.line + 1,
            col: 0,
        },
    }
}

/// The attribute at `at` skipped: up to its closing `]`. Whether it is
/// `#[cfg(test)]` too.
fn skip_attribute(lines: &[Line], at: At) -> (At, bool) {
    let mut depth = 0_i32;
    let mut text = String::new();
    let mut here = at;
    while here.line < lines.len() {
        let code = &lines[here.line].code;
        while here.col < code.len() {
            let c = code[here.col];
            here.col += 1;
            text.push(c);
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth <= 0 {
                    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                    return (here, compact.contains("cfg(test)"));
                }
            }
        }
        here = At {
            line: here.line + 1,
            col: 0,
        };
    }
    (here, false)
}

/// Reads the items from `from` to the end of the container: the lines of
/// their shape, and where the container ends.
fn items(
    lines: &[Line],
    from: At,
    container: Container,
    visibility: Visibility,
) -> (Vec<String>, At) {
    let indent = if container == Container::File {
        ""
    } else {
        "    "
    };
    let mut out = Vec::new();
    let mut doc: Option<String> = None;
    let mut test_only = false;
    let mut at = from;
    loop {
        at = next_code(lines, at);
        if at.line >= lines.len() {
            return (out, at);
        }
        if let Some(text) = &lines[at.line].doc {
            if doc.is_none() && !text.is_empty() {
                doc = Some(text.clone());
            }
            at = At {
                line: at.line + 1,
                col: 0,
            };
            continue;
        }
        let text = rest(lines, at);
        let trimmed = text.trim_start();
        if container != Container::File && trimmed.starts_with('}') {
            return (
                out,
                At {
                    line: at.line,
                    col: at.col + 1,
                },
            );
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            let (after, cfg_test) = skip_attribute(lines, at);
            test_only |= cfg_test;
            at = after;
            continue;
        }
        let item = head(&peek(lines, at), visibility);
        let said_doc = doc.take();
        if std::mem::take(&mut test_only) {
            at = skip_item(lines, at);
            continue;
        }
        // An impl's methods go without their doc: they measured a fifth of
        // the engine's shape, and a trait's are what a unit implements.
        let doc_line = said_doc
            .filter(|_| visibility == Visibility::External && container != Container::Impl)
            .map(|doc| format!("{indent}/// {doc}"));
        let reading = Reading {
            lines,
            container,
            visibility,
            indent,
        };
        at = reading.item(at, &item, doc_line, &mut out);
    }
}

/// What [`items`] reads in, for one item.
struct Reading<'a> {
    lines: &'a [Line],
    container: Container,
    visibility: Visibility,
    indent: &'a str,
}

impl Reading<'_> {
    /// The shape of the item at `at` pushed on `out`, its doc line first;
    /// where the next item starts.
    fn item(&self, at: At, item: &Head, doc: Option<String>, out: &mut Vec<String>) -> At {
        let lines = self.lines;
        let in_trait = self.container == Container::Trait;
        let file = self.container == Container::File;
        match item.kind {
            "impl" if file => self.nested(at, Container::Impl, None, out),
            "trait" if file && item.kept => self.nested(at, Container::Trait, doc, out),
            "struct" | "enum" | "union" if file && item.kept => {
                members(lines, at, item.kind == "enum", out)
            }
            "fn" if item.kept || in_trait => {
                let Some(signature) = join(lines, at, Until::BodyOrEnd) else {
                    return skip_item(lines, at);
                };
                out.extend(doc);
                let (mark, after) = match (in_trait, signature.end) {
                    (true, '{') => (" { … }", past_block(lines, signature.after)),
                    (true, _) => (";", signature.after),
                    (false, '{') => ("", past_block(lines, signature.after)),
                    (false, _) => ("", signature.after),
                };
                out.push(format!("{}{}{mark}", self.indent, signature.text));
                after
            }
            "const" | "static" | "type" if item.kept || in_trait => {
                let Some(signature) = join(lines, at, Until::End) else {
                    return skip_item(lines, at);
                };
                out.extend(doc);
                let end = if in_trait { ";" } else { "" };
                out.push(format!("{}{}{end}", self.indent, constant(&signature.text)));
                signature.after
            }
            "use" | "mod" if file && item.kept => {
                let until = if item.kind == "use" {
                    Until::End
                } else {
                    Until::BodyOrEnd
                };
                let Some(signature) = join(lines, at, until) else {
                    return skip_item(lines, at);
                };
                out.extend(doc.filter(|_| item.kind == "mod"));
                out.push(signature.text);
                if signature.end == '{' {
                    past_block(lines, signature.after)
                } else {
                    signature.after
                }
            }
            _ => skip_item(lines, at),
        }
    }

    /// An `impl` or a `trait` at `at`: its header and the members `inner`
    /// keeps, an impl with none said not at all.
    fn nested(&self, at: At, inner: Container, doc: Option<String>, out: &mut Vec<String>) -> At {
        let Some(header) = join(self.lines, at, Until::BodyOrEnd) else {
            return skip_item(self.lines, at);
        };
        if header.end != '{' {
            return header.after;
        }
        let (members, after) = items(self.lines, header.after, inner, self.visibility);
        if inner == Container::Trait || !members.is_empty() {
            out.extend(doc);
            out.push(format!("{} {{", header.text));
            out.extend(members);
            out.push("}".to_string());
        }
        after
    }
}

/// A `struct`, `enum` or `union` at `at`: its header and, in a block, its
/// variants or its public fields, the others counted.
fn members(lines: &[Line], at: At, variants: bool, out: &mut Vec<String>) -> At {
    let Some(header) = join(lines, at, Until::BodyOrEnd) else {
        return skip_item(lines, at);
    };
    if header.end == ';' {
        out.push(header.text);
        return header.after;
    }
    let mut kept = Vec::new();
    let mut hidden = 0;
    let mut here = header.after;
    loop {
        here = next_code(lines, here);
        if here.line >= lines.len() {
            break;
        }
        if lines[here.line].doc.is_some() {
            here = At {
                line: here.line + 1,
                col: 0,
            };
            continue;
        }
        let text = rest(lines, here);
        if text.trim_start().starts_with("#[") {
            here = skip_attribute(lines, here).0;
            continue;
        }
        let Some(member) = join(lines, here, Until::Member) else {
            here = At {
                line: lines.len(),
                col: 0,
            };
            break;
        };
        here = member.after;
        if !member.text.is_empty() {
            if variants || member.text.starts_with("pub ") {
                kept.push(format!("    {},", member.text));
            } else {
                hidden += 1;
            }
        }
        if member.end == '}' {
            break;
        }
    }
    out.push(format!("{} {{", header.text));
    out.extend(kept);
    if hidden > 0 {
        // Said rather than left out: a struct that looks fieldless reads as
        // one you can build with `Self {}`.
        out.push(format!("    /* {hidden} private field(s) */"));
    }
    out.push("}".to_string());
    here
}

/// The public shape of `source`, as `visibility` reads it: see the module.
#[must_use]
pub fn shape(source: &str, visibility: Visibility) -> String {
    let lines = clean(source);
    items(&lines, At { line: 0, col: 0 }, Container::File, visibility)
        .0
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn external(source: &str) -> String {
        shape(source, Visibility::External)
    }

    #[test]
    fn a_trait_keeps_every_method_and_marks_the_defaulted_ones() {
        let shape = external(
            "/// A projection.\n#[async_trait]\npub trait ProjectPg: Send + Sync {\n    /// Writes.\n    async fn project(\n        &self,\n        write: &Write,\n    ) -> Result<(), Error>;\n\n    /// Warms.\n    async fn warm(&self) -> Result<(), Error> {\n        Ok(())\n    }\n}\n",
        );
        assert!(shape.contains("/// A projection."), "{shape}");
        assert!(
            shape.contains("pub trait ProjectPg: Send + Sync {"),
            "{shape}"
        );
        assert!(
            shape.contains("    async fn project(&self, write: &Write) -> Result<(), Error>;"),
            "{shape}"
        );
        assert!(
            shape.contains("    async fn warm(&self) -> Result<(), Error> { … }"),
            "{shape}"
        );
        assert!(shape.contains("    /// Writes."), "{shape}");
        assert!(!shape.contains("Ok(())"), "no default body leaks: {shape}");
    }

    #[test]
    fn a_signature_over_several_lines_is_joined_into_one() {
        let shape = shape(
            "pub fn extra_for(\n    a: &str,\n    b: Option<&str>,\n) -> String {\n    String::new()\n}\n",
            Visibility::Crate,
        );
        assert_eq!(
            shape,
            "pub fn extra_for(a: &str, b: Option<&str>) -> String"
        );
    }

    #[test]
    fn a_public_constant_is_kept_and_a_long_value_cut() {
        let shape = shape(
            &format!(
                "pub const BUDGET: usize = 12_000;\npub const LONG: [u16; 60] = [{}];\npub const MARK: &str = \"{{ a brace }}\";\n",
                "1000, ".repeat(60)
            ),
            Visibility::Crate,
        );
        assert!(
            shape.contains("pub const BUDGET: usize = 12_000"),
            "{shape}"
        );
        assert!(shape.contains("pub const LONG: [u16; 60] = …"), "{shape}");
        // A string's text is emptied before the scan: its braces never count.
        assert!(shape.contains("pub const MARK: &str = \"…\""), "{shape}");
    }

    #[test]
    fn a_crate_restricted_item_is_a_dependency_s_secret_and_a_neighbour_s_api() {
        let source = "pub(crate) fn inner() -> u8 { 1 }\npub fn outer() -> u8 { 2 }\n";
        let outside = external(source);
        assert!(!outside.contains("inner"), "{outside}");
        assert!(outside.contains("pub fn outer() -> u8"), "{outside}");
        let inside = shape(source, Visibility::Crate);
        assert!(inside.contains("pub(crate) fn inner() -> u8"), "{inside}");
    }

    #[test]
    fn a_test_module_and_private_bodies_are_skipped_whole() {
        let shape = external(
            "fn helper() {\n    impl Hidden { pub fn leak() {} }\n}\npub fn kept() {}\n#[cfg(test)]\nmod tests {\n    impl Fixture {\n        pub fn leak_too() {}\n    }\n}\n",
        );
        assert_eq!(shape, "pub fn kept()");
    }

    #[test]
    fn a_brace_or_a_comment_in_a_string_never_miscounts() {
        let shape = shape(
            "pub fn a() -> &'static str {\n    \"{ // not a comment\"\n}\npub const C: char = '{';\npub fn b(r: &str) { let _ = r#\"}\"#; }\npub fn c() {}\n",
            Visibility::Crate,
        );
        assert!(shape.contains("pub fn a() -> &'static str"), "{shape}");
        assert!(shape.contains("pub fn b(r: &str)"), "{shape}");
        assert!(shape.contains("pub fn c()"), "{shape}");
    }

    #[test]
    fn a_re_export_and_a_module_are_part_of_the_api() {
        let shape = external(
            "pub mod plugins;\n#[cfg(feature = \"postgres\")]\npub use tokio_postgres;\npub use crate::{a::B, c::D};\n",
        );
        assert!(shape.contains("pub mod plugins"), "{shape}");
        assert!(shape.contains("pub use tokio_postgres"), "{shape}");
        assert!(shape.contains("pub use crate::{a::B, c::D}"), "{shape}");
    }

    #[test]
    fn an_enum_variant_with_fields_is_one_line() {
        let shape = external(
            "pub enum Event {\n    Moved {\n        from: u8,\n        to: u8,\n    },\n    Stopped(String),\n}\n",
        );
        assert!(shape.contains("    Moved { from: u8, to: u8 },"), "{shape}");
        assert!(shape.contains("    Stopped(String),"), "{shape}");
    }

    #[test]
    fn a_comma_inside_generics_never_cuts_a_field() {
        let shape = external(
            "pub struct P {\n    pub a: BTreeMap<String, Value>,\n    pub base: &'a BTreeMap<String, Value>,\n    pub f: Box<dyn Fn(u8, u8) -> Result<(), E>>,\n    pub buf: [u8; 1 << 4],\n    pub b: u8,\n}\npub enum K {\n    A = 1 << 2,\n    B,\n}\n",
        );
        assert!(
            shape.contains("    pub a: BTreeMap<String, Value>,"),
            "{shape}"
        );
        assert!(
            shape.contains("    pub base: &'a BTreeMap<String, Value>,"),
            "{shape}"
        );
        assert!(
            shape.contains("    pub f: Box<dyn Fn(u8, u8) -> Result<(), E>>,"),
            "{shape}"
        );
        assert!(shape.contains("    pub buf: [u8; 1 << 4],"), "{shape}");
        assert!(shape.contains("    pub b: u8,"), "{shape}");
        assert!(!shape.contains("private field"), "{shape}");
        assert!(shape.contains("    A = 1 << 2,"), "{shape}");
        assert!(shape.contains("    B,"), "{shape}");
    }

    #[test]
    fn a_generic_default_and_a_value_s_comparison_keep_their_item() {
        let shape = shape(
            "pub struct S<T = Vec<u8>> {\n    pub t: T,\n}\npub const OK: bool = 1 < 2;\npub fn after() {}\n",
            Visibility::Crate,
        );
        assert!(shape.contains("pub struct S<T = Vec<u8>> {"), "{shape}");
        assert!(shape.contains("pub const OK: bool = 1 < 2"), "{shape}");
        assert!(shape.contains("pub fn after()"), "{shape}");
    }

    /// Whether the generics of `line` balance as the reader counts them:
    /// outside brackets and braces, before a value's `=`, `->` and `=>`
    /// closing nothing.
    fn angles_balance(line: &str) -> bool {
        let chars: Vec<char> = line.chars().collect();
        let (mut brackets, mut braces, mut angles) = (0_i32, 0_i32, 0_i32);
        for (i, &c) in chars.iter().enumerate() {
            let before = i.checked_sub(1).and_then(|b| chars.get(b)).copied();
            let after = chars.get(i + 1).copied();
            let outside = brackets == 0 && braces == 0;
            match c {
                '(' | '[' => brackets += 1,
                ')' | ']' => brackets -= 1,
                '{' => braces += 1,
                '}' => braces -= 1,
                '<' if outside => angles += 1,
                '>' if outside && !matches!(before, Some('-' | '=')) => angles -= 1,
                '=' if outside
                    && angles == 0
                    && !matches!(
                        before,
                        Some('=' | '!' | '<' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^')
                    )
                    && !matches!(after, Some('=' | '>')) =>
                {
                    return true;
                }
                _ => {}
            }
            if angles < 0 {
                return false;
            }
        }
        angles == 0
    }

    /// A type with generics, nested up to `depth`.
    fn a_type(depth: u32) -> impl Strategy<Value = String> {
        let leaf = prop_oneof![
            Just("u8".to_string()),
            Just("String".to_string()),
            Just("&'a str".to_string()),
        ];
        leaf.prop_recursive(depth, 16, 3, |inner| {
            prop_oneof![
                (inner.clone(), inner.clone()).prop_map(|(k, v)| format!("BTreeMap<{k}, {v}>")),
                (inner.clone(), inner.clone()).prop_map(|(t, e)| format!("Result<{t}, {e}>")),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, r)| format!("Box<dyn Fn({a}) -> {r}>")),
                (inner.clone(), inner).prop_map(|(a, b)| format!("({a}, {b})")),
            ]
        })
    }

    proptest! {
        #[test]
        fn any_text_shapes_without_panicking_and_every_signature_balances(source in ".{0,400}") {
            for visibility in [Visibility::Crate, Visibility::External] {
                let shaped = shape(&source, visibility);
                for line in shaped.lines().filter(|l| !l.trim_start().starts_with("///")) {
                    prop_assert_eq!(line.matches('(').count(), line.matches(')').count(), "{}", line);
                    prop_assert!(angles_balance(line), "{}", line);
                }
            }
        }

        #[test]
        fn every_public_field_of_any_generic_type_is_kept_whole(types in proptest::collection::vec(a_type(3), 1..6)) {
            let fields: Vec<String> = types.iter().enumerate().map(|(i, ty)| format!("pub f{i}: {ty}")).collect();
            let source = format!("pub struct P<'a> {{\n{}\n}}\n", fields.iter().map(|f| format!("    {f},")).collect::<Vec<_>>().join("\n"));
            let shaped = external(&source);
            for field in &fields {
                prop_assert!(shaped.contains(&format!("    {field},")), "{} in {}", field, shaped);
            }
            prop_assert!(!shaped.contains("private field"), "{}", shaped);
            for line in shaped.lines() {
                prop_assert!(angles_balance(line), "{}", line);
            }
        }

        #[test]
        fn a_signature_cut_anywhere_across_lines_reads_as_one(cuts in proptest::collection::vec(0usize..60, 0..6)) {
            let whole = "pub fn wide(alpha: &str, beta: Option<u8>, gamma: Vec<String>) -> Result<(), String>";
            let mut text = whole.to_string();
            let mut points: Vec<usize> = cuts.into_iter().filter(|&c| c < whole.len() && whole.as_bytes()[c] == b' ').collect();
            points.sort_unstable();
            points.dedup();
            for point in points.into_iter().rev() {
                text.replace_range(point..=point, "\n    ");
            }
            let shaped = shape(&format!("{text} {{\n    todo()\n}}\n"), Visibility::Crate);
            prop_assert_eq!(shaped, whole);
        }
    }
}

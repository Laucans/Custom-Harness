//! A Postgres schema, read from its DDL — what `pg_dump --schema-only`
//! writes, or a hand-written `CREATE TABLE` file.
//!
//! Not a SQL parser: a tokenizer that knows strings, quoted names, dollar
//! quotes and comments, then five statement shapes — `CREATE TABLE`,
//! `ALTER TABLE … ADD CONSTRAINT / ALTER COLUMN`, `CREATE [UNIQUE] INDEX`,
//! `CREATE TYPE … AS ENUM`, `COMMENT ON TABLE / COLUMN`. Every other
//! statement (functions, sequences, grants, owners) is skipped whole, and
//! nothing here panics on a file it does not understand: it reads less.

use std::collections::BTreeMap;

/// The tables of a schema, and its enumerated types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    /// In the order the file creates them.
    pub tables: Vec<SqlTable>,
    /// `CREATE TYPE … AS ENUM`, by type name, values in order.
    pub enums: BTreeMap<String, Vec<String>>,
}

/// One table as the database has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlTable {
    /// `public.` dropped; another schema stays: `audit.event`.
    pub name: String,
    /// `COMMENT ON TABLE`.
    pub comment: Option<String>,
    /// In declaration order.
    pub columns: Vec<SqlColumn>,
    /// Column and table constraints alike, column ones made table ones.
    pub constraints: Vec<Constraint>,
    /// `CREATE INDEX` on this table.
    pub indexes: Vec<Index>,
}

/// One column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlColumn {
    /// The column's name, folded to lower case unless it was quoted.
    pub name: String,
    /// As written: `character varying(40)`, `timestamp with time zone`.
    pub data_type: String,
    /// False under `NOT NULL`.
    pub nullable: bool,
    /// The `DEFAULT` expression, as written.
    pub default: Option<String>,
    /// `GENERATED …`, as written.
    pub generated: Option<String>,
    /// `COMMENT ON COLUMN`.
    pub comment: Option<String>,
}

/// A constraint, named or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// `CONSTRAINT <name>`, when given.
    pub name: Option<String>,
    /// What it holds.
    pub rule: Rule,
}

/// What a constraint holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// `PRIMARY KEY (columns)`.
    Primary(Vec<String>),
    /// `UNIQUE (columns)`.
    Unique(Vec<String>),
    /// `FOREIGN KEY (columns) REFERENCES table (references)`.
    Foreign {
        /// The columns of this table.
        columns: Vec<String>,
        /// The referenced table, named as [`SqlTable::name`].
        table: String,
        /// Its columns; empty when the DDL leaves them implicit (its key).
        references: Vec<String>,
        /// `ON DELETE …`, lower case.
        on_delete: Option<String>,
        /// `ON UPDATE …`, lower case.
        on_update: Option<String>,
    },
    /// `CHECK (expression)`, the expression as written.
    Check(String),
    /// Anything else (`EXCLUDE …`), as written.
    Other(String),
}

/// An index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// Its name, when given.
    pub name: Option<String>,
    /// `CREATE UNIQUE INDEX`.
    pub unique: bool,
    /// A column name, or an expression as written.
    pub columns: Vec<String>,
}

/// Reads a schema out of DDL. Never fails: what it cannot read, it skips.
#[must_use]
pub fn parse(sql: &str) -> Schema {
    let tokens = tokenize(sql);
    let mut schema = Schema::default();
    for statement in tokens.split(|t| t.is_punct(';')) {
        if statement.is_empty() {
            continue;
        }
        let mut c = Cursor::new(statement, sql);
        if c.is_word("create") {
            create(&mut c, &mut schema);
        } else if c.eat_seq(&["alter", "table"]) {
            alter_table(&mut c, &mut schema);
        } else if c.eat_seq(&["comment", "on"]) {
            comment(&mut c, &mut schema);
        }
    }
    schema
}

// ---- tokens --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A keyword, an unquoted name or a number — folded to lower case.
    Word,
    /// A `"quoted"` name, as written inside the quotes.
    Quoted,
    /// A `'string'` or a `$$dollar$$` string, its content.
    Text,
    /// One character of punctuation.
    Punct,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    text: String,
    /// Byte offsets into the source, quotes included.
    start: usize,
    end: usize,
}

impl Token {
    fn is_punct(&self, p: char) -> bool {
        self.kind == Kind::Punct && self.text.len() == 1 && self.text.starts_with(p)
    }

    fn is_word(&self, w: &str) -> bool {
        self.kind == Kind::Word && self.text == w
    }

    /// A name: a word or a quoted name.
    fn ident(&self) -> Option<String> {
        matches!(self.kind, Kind::Word | Kind::Quoted).then(|| self.text.clone())
    }
}

const fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// Splits the source into tokens; comments and whitespace go.
///
/// Every token boundary sits on an ASCII byte or the end, so every
/// `start..end` is a valid slice of `sql`.
fn tokenize(sql: &str) -> Vec<Token> {
    let bytes = sql.as_bytes();
    let at = |i: usize| bytes.get(i).copied();
    let slice = |a: usize, b: usize| sql.get(a..b).unwrap_or_default().to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = at(i) {
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && at(i + 1) == Some(b'-') {
            while at(i).is_some_and(|c| c != b'\n') {
                i += 1;
            }
        } else if c == b'/' && at(i + 1) == Some(b'*') {
            i += 2;
            let mut depth = 1_u32;
            while depth > 0 {
                match (at(i), at(i + 1)) {
                    (None, _) => break,
                    (Some(b'*'), Some(b'/')) => {
                        depth -= 1;
                        i += 2;
                    }
                    (Some(b'/'), Some(b'*')) => {
                        depth = depth.saturating_add(1);
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
        } else if c == b'\'' || c == b'"' {
            // A doubled quote is the quote itself.
            i += 1;
            let mut closed = false;
            while let Some(d) = at(i) {
                if d == c {
                    if at(i + 1) == Some(c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            let inner_end = if closed { i - 1 } else { i };
            let quote = char::from(c).to_string();
            let text = slice(start + 1, inner_end).replace(&quote.repeat(2), &quote);
            let kind = if c == b'\'' { Kind::Text } else { Kind::Quoted };
            out.push(Token {
                kind,
                text,
                start,
                end: i,
            });
        } else if let Some(tag_end) = dollar_tag(bytes, i) {
            let tag = slice(i, tag_end);
            let body = tag_end;
            let close = sql.get(body..).and_then(|rest| rest.find(&tag));
            let (text_end, end) = close.map_or((sql.len(), sql.len()), |off| {
                (body + off, body + off + tag.len())
            });
            out.push(Token {
                kind: Kind::Text,
                text: slice(body, text_end),
                start,
                end,
            });
            i = end;
        } else if is_word_byte(c) && c != b'$' {
            while at(i)
                .is_some_and(|c| is_word_byte(c) || (c == b'.' && start_is_digit(bytes, start)))
            {
                i += 1;
            }
            out.push(Token {
                kind: Kind::Word,
                text: slice(start, i).to_lowercase(),
                start,
                end: i,
            });
        } else {
            i += 1;
            out.push(Token {
                kind: Kind::Punct,
                text: char::from(c).to_string(),
                start,
                end: i,
            });
        }
    }
    out
}

fn start_is_digit(bytes: &[u8], start: usize) -> bool {
    bytes.get(start).is_some_and(u8::is_ascii_digit)
}

/// `$tag$` or `$$` opening at `i`: the offset just past it.
fn dollar_tag(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'$') {
        return None;
    }
    let mut j = i + 1;
    if bytes.get(j).is_some_and(u8::is_ascii_digit) {
        return None; // `$1`, a parameter
    }
    while bytes
        .get(j)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        j += 1;
    }
    (bytes.get(j) == Some(&b'$')).then_some(j + 1)
}

// ---- a cursor over one statement ------------------------------------------------

struct Cursor<'a> {
    toks: &'a [Token],
    pos: usize,
    src: &'a str,
}

impl<'a> Cursor<'a> {
    const fn new(toks: &'a [Token], src: &'a str) -> Self {
        Self { toks, pos: 0, src }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.toks.get(self.pos)
    }

    const fn done(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn is_word(&self, w: &str) -> bool {
        self.peek().is_some_and(|t| t.is_word(w))
    }

    fn is_punct(&self, p: char) -> bool {
        self.peek().is_some_and(|t| t.is_punct(p))
    }

    fn eat(&mut self, w: &str) -> bool {
        let yes = self.is_word(w);
        if yes {
            self.pos += 1;
        }
        yes
    }

    /// All the words in a row, or nothing eaten.
    fn eat_seq(&mut self, words: &[&str]) -> bool {
        let yes = words
            .iter()
            .enumerate()
            .all(|(k, w)| self.toks.get(self.pos + k).is_some_and(|t| t.is_word(w)));
        if yes {
            self.pos += words.len();
        }
        yes
    }

    fn ident(&mut self) -> Option<String> {
        let name = self.peek()?.ident()?;
        self.pos += 1;
        Some(name)
    }

    /// `a.b.c`, each part a name.
    fn qualified(&mut self) -> Option<Vec<String>> {
        let mut parts = vec![self.ident()?];
        while self.is_punct('.') {
            self.pos += 1;
            parts.push(self.ident()?);
        }
        Some(parts)
    }

    /// The tokens inside the parentheses that open here, the cursor moved
    /// past the closing one. `None`, and nothing eaten, without a `(`.
    fn group(&mut self) -> Option<&'a [Token]> {
        if !self.is_punct('(') {
            return None;
        }
        let open = self.pos;
        let toks = self.toks;
        let mut depth = 0_usize;
        for (k, t) in toks.iter().enumerate().skip(open) {
            if t.is_punct('(') {
                depth += 1;
            } else if t.is_punct(')') {
                depth -= 1;
                if depth == 0 {
                    self.pos = k + 1;
                    return toks.get(open + 1..k);
                }
            }
        }
        self.pos = self.toks.len();
        self.toks.get(open + 1..)
    }

    /// `(a, b)` as names; an expression item comes back as written.
    fn name_list(&mut self) -> Vec<String> {
        self.group()
            .map(|inner| {
                split_commas(inner)
                    .into_iter()
                    .map(|item| match item {
                        [one] => one.ident().unwrap_or_else(|| span(self.src, item)),
                        _ => span(self.src, item),
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What is left, as written.
    fn rest(&self) -> String {
        span(self.src, self.toks.get(self.pos..).unwrap_or_default())
    }

    /// The tokens up to the next word of `stops` at depth zero.
    fn until(&mut self, stops: &[&str]) -> &'a [Token] {
        let from = self.pos;
        let mut depth = 0_usize;
        while let Some(t) = self.peek() {
            if t.is_punct('(') || t.is_punct('[') {
                depth += 1;
            } else if t.is_punct(')') || t.is_punct(']') {
                depth = depth.saturating_sub(1);
            } else if depth == 0 && t.kind == Kind::Word && stops.contains(&t.text.as_str()) {
                break;
            }
            self.pos += 1;
        }
        self.toks.get(from..self.pos).unwrap_or_default()
    }
}

/// The source behind a run of tokens, whitespace collapsed.
fn span(src: &str, toks: &[Token]) -> String {
    let (Some(first), Some(last)) = (toks.first(), toks.last()) else {
        return String::new();
    };
    src.get(first.start..last.end)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits on the commas at depth zero.
fn split_commas(toks: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut from = 0;
    for (k, t) in toks.iter().enumerate() {
        if t.is_punct('(') || t.is_punct('[') {
            depth += 1;
        } else if t.is_punct(')') || t.is_punct(']') {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && t.is_punct(',') {
            out.push(toks.get(from..k).unwrap_or_default());
            from = k + 1;
        }
    }
    out.push(toks.get(from..).unwrap_or_default());
    out.retain(|item| !item.is_empty());
    out
}

/// A table's name as the schema keys it: `public.` dropped.
fn table_name(parts: &[String]) -> String {
    match parts {
        [schema, name] if schema == "public" => name.clone(),
        _ => parts.join("."),
    }
}

// ---- statements -------------------------------------------------------------------

fn create(c: &mut Cursor<'_>, schema: &mut Schema) {
    c.bump(); // create
    let mut unique = false;
    // `create [or replace] [global|local] [temp|temporary|unlogged] table`,
    // `create [unique] index`, `create type`.
    while let Some(t) = c.peek() {
        match t.text.as_str() {
            "or" | "replace" | "global" | "local" | "temp" | "temporary" | "unlogged" => {
                c.bump();
            }
            "unique" => {
                unique = true;
                c.bump();
            }
            _ => break,
        }
    }
    if c.eat("table") {
        create_table(c, schema);
    } else if c.eat("index") {
        create_index(c, schema, unique);
    } else if c.eat("type") {
        create_enum(c, schema);
    }
}

/// The words that end a column's type and start its constraints.
const COLUMN_WORDS: [&str; 10] = [
    "constraint",
    "not",
    "null",
    "default",
    "primary",
    "unique",
    "references",
    "check",
    "generated",
    "collate",
];

/// The words that end a `GENERATED …` clause — not `default`, which
/// `GENERATED BY DEFAULT AS IDENTITY` carries.
const GENERATED_STOPS: [&str; 7] = [
    "constraint",
    "not",
    "primary",
    "unique",
    "references",
    "check",
    "collate",
];

/// The words that open a table constraint.
const TABLE_CONSTRAINT_WORDS: [&str; 7] = [
    "constraint",
    "primary",
    "unique",
    "foreign",
    "check",
    "exclude",
    "like",
];

fn create_table(c: &mut Cursor<'_>, schema: &mut Schema) {
    c.eat_seq(&["if", "not", "exists"]);
    let Some(parts) = c.qualified() else { return };
    // `partition of`, `as select`, `of type`: no column list to read.
    let Some(body) = c.group() else { return };
    let mut table = SqlTable {
        name: table_name(&parts),
        ..SqlTable::default()
    };
    for item in split_commas(body) {
        let mut ic = Cursor::new(item, c.src);
        if item.first().is_some_and(|t| {
            t.kind == Kind::Word && TABLE_CONSTRAINT_WORDS.contains(&t.text.as_str())
        }) {
            if let Some(constraint) = table_constraint(&mut ic) {
                table.constraints.push(constraint);
            }
        } else if let Some((column, constraints)) = column(&mut ic) {
            table.columns.push(column);
            table.constraints.extend(constraints);
        }
    }
    match schema.tables.iter_mut().find(|t| t.name == table.name) {
        Some(known) => *known = table,
        None => schema.tables.push(table),
    }
}

/// A column definition: name, type, then its constraints.
fn column(c: &mut Cursor<'_>) -> Option<(SqlColumn, Vec<Constraint>)> {
    let name = c.ident()?;
    let data_type = span(c.src, c.until(&COLUMN_WORDS));
    if data_type.is_empty() {
        return None;
    }
    let mut col = SqlColumn {
        name: name.clone(),
        data_type,
        nullable: true,
        ..SqlColumn::default()
    };
    let mut constraints = Vec::new();
    let mut named: Option<String> = None;
    while !c.done() {
        let before = c.pos;
        if c.eat("constraint") {
            named = c.ident();
            continue;
        }
        if c.eat_seq(&["not", "null"]) {
            col.nullable = false;
        } else if c.eat("null") {
            col.nullable = true;
        } else if c.eat("default") {
            // `DEFAULT NULL` is an expression of its own.
            let expr = if c.is_word("null") {
                c.bump().map(|t| t.text.clone()).unwrap_or_default()
            } else {
                span(c.src, c.until(&COLUMN_WORDS))
            };
            col.default = (!expr.is_empty()).then_some(expr);
        } else if c.eat_seq(&["primary", "key"]) {
            constraints.push(Constraint {
                name: named.take(),
                rule: Rule::Primary(vec![name.clone()]),
            });
        } else if c.eat("unique") {
            constraints.push(Constraint {
                name: named.take(),
                rule: Rule::Unique(vec![name.clone()]),
            });
        } else if c.eat("references") {
            if let Some(rule) = references(c, vec![name.clone()]) {
                constraints.push(Constraint {
                    name: named.take(),
                    rule,
                });
            }
        } else if c.eat("check") {
            let expr = c.group().map(|g| span(c.src, g)).unwrap_or_default();
            constraints.push(Constraint {
                name: named.take(),
                rule: Rule::Check(expr),
            });
        } else if c.is_word("generated") {
            let from = c.pos;
            c.bump();
            c.until(&GENERATED_STOPS);
            col.generated = Some(span(c.src, c.toks.get(from..c.pos).unwrap_or_default()));
        } else if c.eat("collate") {
            c.qualified();
        }
        if c.pos == before {
            c.bump();
        }
    }
    Some((col, constraints))
}

/// After `REFERENCES`: the table, its columns, the actions.
fn references(c: &mut Cursor<'_>, columns: Vec<String>) -> Option<Rule> {
    let parts = c.qualified()?;
    let refs = c.name_list();
    let mut on_delete = None;
    let mut on_update = None;
    while !c.done() {
        if c.eat_seq(&["on", "delete"]) {
            on_delete = action(c);
        } else if c.eat_seq(&["on", "update"]) {
            on_update = action(c);
        } else if c.eat_seq(&["not", "deferrable"]) {
        } else if c
            .peek()
            .is_some_and(|t| t.kind == Kind::Word && COLUMN_WORDS.contains(&t.text.as_str()))
        {
            break; // the column's next constraint
        } else {
            c.bump(); // match full, deferrable, initially deferred…
        }
    }
    Some(Rule::Foreign {
        columns,
        table: table_name(&parts),
        references: refs,
        on_delete,
        on_update,
    })
}

/// `cascade`, `restrict`, `no action`, `set null`, `set default`.
fn action(c: &mut Cursor<'_>) -> Option<String> {
    let first = c.bump()?.text.clone();
    if first == "set" || first == "no" {
        let second = c.bump().map(|t| t.text.clone()).unwrap_or_default();
        return Some(format!("{first} {second}"));
    }
    Some(first)
}

/// A table constraint, from `CONSTRAINT` or its first keyword.
fn table_constraint(c: &mut Cursor<'_>) -> Option<Constraint> {
    let name = if c.eat("constraint") { c.ident() } else { None };
    let rule = if c.eat_seq(&["primary", "key"]) {
        Rule::Primary(c.name_list())
    } else if c.eat("unique") {
        if !c.eat_seq(&["nulls", "not", "distinct"]) {
            c.eat_seq(&["nulls", "distinct"]);
        }
        Rule::Unique(c.name_list())
    } else if c.eat_seq(&["foreign", "key"]) {
        let columns = c.name_list();
        if !c.eat("references") {
            return None;
        }
        references(c, columns)?
    } else if c.eat("check") {
        Rule::Check(c.group().map(|g| span(c.src, g)).unwrap_or_default())
    } else {
        let rest = c.rest();
        if rest.is_empty() {
            return None;
        }
        Rule::Other(rest)
    };
    Some(Constraint { name, rule })
}

fn alter_table(c: &mut Cursor<'_>, schema: &mut Schema) {
    c.eat_seq(&["if", "exists"]);
    c.eat("only");
    let Some(parts) = c.qualified() else { return };
    let name = table_name(&parts);
    let Some(table) = schema.tables.iter_mut().find(|t| t.name == name) else {
        return;
    };
    let rest = c.toks.get(c.pos..).unwrap_or_default();
    for item in split_commas(rest) {
        let mut ac = Cursor::new(item, c.src);
        if ac.eat("add") {
            let constraint_ahead = ac.peek().is_some_and(|t| {
                t.kind == Kind::Word && TABLE_CONSTRAINT_WORDS.contains(&t.text.as_str())
            });
            if constraint_ahead {
                if let Some(constraint) = table_constraint(&mut ac) {
                    table.constraints.push(constraint);
                }
            } else {
                ac.eat("column");
                ac.eat_seq(&["if", "not", "exists"]);
                if let Some((column, constraints)) = column(&mut ac) {
                    table.columns.push(column);
                    table.constraints.extend(constraints);
                }
            }
        } else if ac.eat("alter") {
            ac.eat("column");
            let Some(col_name) = ac.ident() else { continue };
            let Some(col) = table.columns.iter_mut().find(|k| k.name == col_name) else {
                continue;
            };
            if ac.eat_seq(&["set", "default"]) {
                col.default = Some(ac.rest());
            } else if ac.eat_seq(&["set", "not", "null"]) {
                col.nullable = false;
            } else if ac.eat_seq(&["drop", "not", "null"]) {
                col.nullable = true;
            } else if ac.is_word("add")
                && ac
                    .toks
                    .get(ac.pos + 1)
                    .is_some_and(|t| t.is_word("generated"))
            {
                ac.bump();
                col.generated = Some(ac.rest());
            }
        }
    }
}

fn create_index(c: &mut Cursor<'_>, schema: &mut Schema, unique: bool) {
    c.eat("concurrently");
    c.eat_seq(&["if", "not", "exists"]);
    let name = if c.is_word("on") { None } else { c.ident() };
    if !c.eat("on") {
        return;
    }
    c.eat("only");
    let Some(parts) = c.qualified() else { return };
    if c.eat("using") {
        c.bump();
    }
    let columns = c.name_list();
    let table = table_name(&parts);
    if let Some(t) = schema.tables.iter_mut().find(|t| t.name == table) {
        t.indexes.push(Index {
            name,
            unique,
            columns,
        });
    }
}

fn create_enum(c: &mut Cursor<'_>, schema: &mut Schema) {
    let Some(parts) = c.qualified() else { return };
    if !c.eat_seq(&["as", "enum"]) {
        return;
    }
    let values = c
        .group()
        .map(|inner| {
            inner
                .iter()
                .filter(|t| t.kind == Kind::Text)
                .map(|t| t.text.clone())
                .collect()
        })
        .unwrap_or_default();
    schema.enums.insert(table_name(&parts), values);
}

fn comment(c: &mut Cursor<'_>, schema: &mut Schema) {
    let on_table = c.eat("table");
    if !on_table && !c.eat("column") {
        return;
    }
    let Some(mut parts) = c.qualified() else {
        return;
    };
    if !c.eat("is") {
        return;
    }
    let Some(text) = c
        .peek()
        .filter(|t| t.kind == Kind::Text)
        .map(|t| t.text.clone())
    else {
        return; // `IS NULL` drops a comment
    };
    if on_table {
        let name = table_name(&parts);
        if let Some(t) = schema.tables.iter_mut().find(|t| t.name == name) {
            t.comment = Some(text);
        }
        return;
    }
    let Some(col_name) = parts.pop() else { return };
    let name = table_name(&parts);
    if let Some(col) = schema
        .tables
        .iter_mut()
        .find(|t| t.name == name)
        .and_then(|t| t.columns.iter_mut().find(|k| k.name == col_name))
    {
        col.comment = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fmt::Write as _;

    const DUMP: &str = r#"
--
-- PostgreSQL database dump
--
SET statement_timeout = 0;
SELECT pg_catalog.set_config('search_path', '', false);

CREATE TYPE public.class_kind AS ENUM (
    'fighter',
    'wizard'
);

CREATE FUNCTION public.touch() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
  NEW.updated_at = now(); -- a ; inside a body
  RETURN NEW;
END;
$$;

CREATE TABLE public.campaign (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    "Started" timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT campaign_name_check CHECK ((length(name) > 0))
);

ALTER TABLE public.campaign OWNER TO app;

CREATE TABLE public."character" (
    id integer NOT NULL,
    campaign_id uuid,
    class public.class_kind NOT NULL,
    level smallint DEFAULT 1 NOT NULL,
    name character varying(40) NOT NULL
);

CREATE SEQUENCE public.character_id_seq AS integer START WITH 1;
ALTER TABLE ONLY public."character" ALTER COLUMN id SET DEFAULT nextval('public.character_id_seq'::regclass);

ALTER TABLE ONLY public.campaign
    ADD CONSTRAINT campaign_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public."character"
    ADD CONSTRAINT character_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public."character"
    ADD CONSTRAINT character_campaign_id_fkey FOREIGN KEY (campaign_id) REFERENCES public.campaign(id) ON DELETE CASCADE;
CREATE UNIQUE INDEX character_name_idx ON public."character" USING btree (campaign_id, name);
CREATE INDEX character_level_idx ON public."character" USING btree (level);

COMMENT ON TABLE public.campaign IS 'A story the players share';
COMMENT ON COLUMN public."character".name IS 'What the table calls them; it''s unique';
"#;

    fn table<'a>(schema: &'a Schema, name: &str) -> &'a SqlTable {
        schema
            .tables
            .iter()
            .find(|t| t.name == name)
            .expect("the table was parsed")
    }

    #[test]
    fn a_pg_dump_reads_into_tables_columns_and_constraints() {
        let schema = parse(DUMP);
        assert_eq!(
            schema
                .tables
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["campaign", "character"]
        );
        assert_eq!(schema.enums["class_kind"], ["fighter", "wizard"]);

        let campaign = table(&schema, "campaign");
        assert_eq!(
            campaign.comment.as_deref(),
            Some("A story the players share")
        );
        let started = &campaign.columns[2];
        assert_eq!(started.name, "Started", "a quoted name keeps its case");
        assert_eq!(started.data_type, "timestamp with time zone");
        assert_eq!(started.default.as_deref(), Some("now()"));
        assert!(!started.nullable);
        assert!(
            campaign
                .constraints
                .iter()
                .any(|k| k.rule == Rule::Check("(length(name) > 0)".into()))
        );
        assert!(
            campaign
                .constraints
                .iter()
                .any(|k| k.rule == Rule::Primary(vec!["id".into()]))
        );

        let character = table(&schema, "character");
        let id = &character.columns[0];
        assert_eq!(
            id.default.as_deref(),
            Some("nextval('public.character_id_seq'::regclass)")
        );
        assert_eq!(character.columns[1].data_type, "uuid");
        assert!(character.columns[1].nullable);
        assert_eq!(character.columns[4].data_type, "character varying(40)");
        assert_eq!(
            character.columns[4].comment.as_deref(),
            Some("What the table calls them; it's unique")
        );
        assert!(character.constraints.iter().any(|k| k.rule
            == Rule::Foreign {
                columns: vec!["campaign_id".into()],
                table: "campaign".into(),
                references: vec!["id".into()],
                on_delete: Some("cascade".into()),
                on_update: None,
            }));
        assert_eq!(
            character.indexes,
            [
                Index {
                    name: Some("character_name_idx".into()),
                    unique: true,
                    columns: vec!["campaign_id".into(), "name".into()],
                },
                Index {
                    name: Some("character_level_idx".into()),
                    unique: false,
                    columns: vec!["level".into()],
                },
            ]
        );
    }

    #[test]
    fn hand_written_ddl_carries_its_constraints_inline() {
        let schema = parse(
            "create table if not exists item (\n\
               id bigserial primary key,\n\
               owner_id integer references player (id) on delete set null on update cascade not null,\n\
               code text unique check (code <> ''),\n\
               weight numeric(6, 2) default null,\n\
               total integer generated always as (weight * 2) stored,\n\
               primary key (id, code),\n\
               constraint fk_kind foreign key (kind_a, kind_b) references kind\n\
             )",
        );
        let item = table(&schema, "item");
        assert_eq!(item.columns.len(), 5);
        assert!(
            !item.columns[1].nullable,
            "a NOT NULL after REFERENCES is the column's"
        );
        assert_eq!(item.columns[3].data_type, "numeric(6, 2)");
        assert_eq!(item.columns[3].default.as_deref(), Some("null"));
        assert_eq!(
            item.columns[4].generated.as_deref(),
            Some("generated always as (weight * 2) stored")
        );
        let rules: Vec<&Rule> = item.constraints.iter().map(|k| &k.rule).collect();
        assert!(rules.contains(&&Rule::Primary(vec!["id".into()])));
        assert!(rules.contains(&&Rule::Unique(vec!["code".into()])));
        assert!(rules.contains(&&Rule::Check("code <> ''".into())));
        assert!(rules.contains(&&Rule::Primary(vec!["id".into(), "code".into()])));
        assert!(rules.contains(&&Rule::Foreign {
            columns: vec!["owner_id".into()],
            table: "player".into(),
            references: vec!["id".into()],
            on_delete: Some("set null".into()),
            on_update: Some("cascade".into()),
        }));
        assert!(item.constraints.iter().any(|k| k.name.as_deref() == Some("fk_kind")
            && matches!(&k.rule, Rule::Foreign { references, .. } if references.is_empty())));
    }

    #[test]
    fn another_schema_keeps_its_prefix_and_strings_hide_their_semicolons() {
        let schema = parse(
            "CREATE TABLE audit.event (note text DEFAULT 'a;b' /* ; */);\n\
             CREATE TABLE public.x (y int);",
        );
        assert_eq!(
            table(&schema, "audit.event").columns[0].default.as_deref(),
            Some("'a;b'")
        );
        assert_eq!(table(&schema, "x").columns[0].data_type, "int");
    }

    #[test]
    fn what_it_does_not_understand_it_skips() {
        assert_eq!(parse(""), Schema::default());
        assert_eq!(parse("CREATE TABLE ("), Schema::default());
        assert_eq!(
            parse("CREATE TABLE t (a int").tables[0].columns[0].name,
            "a"
        );
        assert_eq!(
            parse("ALTER TABLE nowhere ADD CONSTRAINT k UNIQUE (a);"),
            Schema::default()
        );
    }

    proptest! {
        #[test]
        fn any_text_reads_without_a_panic(sql in "\\PC{0,400}") {
            let _ = parse(&sql);
        }

        #[test]
        fn any_ddl_shaped_text_reads_without_a_panic(
            sql in "(create|table|alter|only|add|constraint|primary|key|references|unique|index|on|comment|is|type|as|enum|[(),.;'\"$ ]|[a-z]{1,3}| ){0,80}"
        ) {
            let _ = parse(&sql);
        }

        #[test]
        fn generated_tables_come_back_as_written(
            tables in prop::collection::btree_map(
                "[a-z][a-z0-9_]{0,8}",
                prop::collection::btree_map(
                    "[a-z][a-z0-9_]{0,8}",
                    (prop::sample::select(vec![
                        "integer", "text", "uuid", "boolean", "numeric(10,2)",
                        "timestamp with time zone", "character varying(40)", "jsonb",
                    ]), any::<bool>()),
                    1..6,
                ),
                1..4,
            )
        ) {
            // Names that are keywords would need quoting; a pg_dump quotes them.
            let quote = |n: &str| format!("\"{n}\"");
            let mut sql = String::new();
            for (name, columns) in &tables {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|(c, (ty, null))| format!("    {} {ty}{}", quote(c), if *null { "" } else { " NOT NULL" }))
                    .collect();
                let _ = writeln!(sql, "CREATE TABLE public.{} (\n{}\n);", quote(name), cols.join(",\n"));
            }
            let schema = parse(&sql);
            prop_assert_eq!(schema.tables.len(), tables.len());
            for (t, (name, columns)) in schema.tables.iter().zip(&tables) {
                prop_assert_eq!(&t.name, name);
                prop_assert_eq!(t.columns.len(), columns.len());
                for (col, (c, (ty, null))) in t.columns.iter().zip(columns) {
                    prop_assert_eq!(&col.name, c);
                    prop_assert_eq!(col.data_type.replace(", ", ","), ty.to_string());
                    prop_assert_eq!(col.nullable, *null);
                }
            }
        }
    }
}

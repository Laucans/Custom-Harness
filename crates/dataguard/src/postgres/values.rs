//! PostgreSQL values and JSON, both ways, without a database: what a
//! Capability's read binds ([`bound`]) and what its rows answer
//! ([`object`]). Pure, so every mapping is tested here; the store only
//! hands the bytes over.
//!
//! A row becomes an object, column name to value. The common types map to
//! their JSON shape: integers, floats (a non-finite one as its PostgreSQL
//! spelling, `"NaN"`), booleans, text, `json`/`jsonb` as is, `uuid` as its
//! hyphenated form, `timestamptz` as RFC 3339 in UTC, `numeric` as a
//! decimal string — JSON numbers would lose its precision — and `NULL` as
//! `null`. Any other type is an error naming its column, never a guess.

use std::error::Error as StdError;
use std::fmt::Write as _;

use bytes::BytesMut;
use serde_json::{Map, Number, Value};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type, to_sql_checked};

use crate::{Error, Result};

/// What a `FromSql` or `ToSql` fails with.
type Failure = Box<dyn StdError + Sync + Send>;

/// Microseconds between the Unix epoch and PostgreSQL's, 2000-01-01.
const EPOCH_2000_MICROS: i128 = 946_684_800_000_000;

/// The bytes of one value as the server sent them: `None` for `NULL`.
pub(super) struct Raw<'a>(pub(super) Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> std::result::Result<Self, Failure> {
        Ok(Self(Some(raw)))
    }

    fn from_sql_null(_ty: &Type) -> std::result::Result<Self, Failure> {
        Ok(Self(None))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// One row as a JSON object: each column's name to its value.
///
/// # Errors
///
/// [`Error::Store`] naming the column whose type has no JSON mapping, or
/// whose bytes do not decode.
pub(super) fn object(columns: &[(&str, &Type)], values: &[Option<&[u8]>]) -> Result<Value> {
    if columns.len() != values.len() {
        return Err(Error::Store(format!(
            "a row of {} value(s) for {} column(s)",
            values.len(),
            columns.len()
        )));
    }
    let mut row = Map::new();
    for ((name, ty), raw) in columns.iter().zip(values) {
        row.insert((*name).to_string(), value(name, ty, *raw)?);
    }
    Ok(Value::Object(row))
}

/// One column's value as JSON.
fn value(column: &str, ty: &Type, raw: Option<&[u8]>) -> Result<Value> {
    let Some(raw) = raw else {
        return Ok(Value::Null);
    };
    let decoded: std::result::Result<Value, Failure> = if *ty == Type::INT2 {
        i16::from_sql(ty, raw).map(Value::from)
    } else if *ty == Type::INT4 {
        i32::from_sql(ty, raw).map(Value::from)
    } else if *ty == Type::INT8 {
        i64::from_sql(ty, raw).map(Value::from)
    } else if *ty == Type::FLOAT4 {
        // Through its shortest text, so `0.1f32` reads `0.1`, not
        // `0.10000000149011612`.
        f32::from_sql(ty, raw).map(|x| float(x.to_string().parse().unwrap_or(f64::NAN)))
    } else if *ty == Type::FLOAT8 {
        f64::from_sql(ty, raw).map(float)
    } else if *ty == Type::BOOL {
        bool::from_sql(ty, raw).map(Value::Bool)
    } else if *ty == Type::JSON || *ty == Type::JSONB {
        Value::from_sql(ty, raw)
    } else if *ty == Type::NUMERIC {
        numeric(raw).map(Value::String).map_err(Failure::from)
    } else if *ty == Type::UUID {
        uuid(raw).map(Value::String).map_err(Failure::from)
    } else if *ty == Type::TIMESTAMPTZ {
        <[u8; 8]>::try_from(raw)
            .map(|bytes| Value::String(rfc3339(i64::from_be_bytes(bytes))))
            .map_err(|_| Failure::from(format!("{} byte(s) for a timestamptz", raw.len())))
    } else if <String as FromSql>::accepts(ty) {
        String::from_sql(ty, raw).map(Value::String)
    } else {
        return Err(Error::Store(format!(
            "column {column}: the type {ty} has no JSON mapping"
        )));
    };
    decoded.map_err(|e| Error::Store(format!("column {column}: {e}")))
}

/// A float as a JSON number, or its PostgreSQL spelling when JSON has none.
fn float(x: f64) -> Value {
    Number::from_f64(x).map_or_else(
        || {
            Value::String(
                if x.is_nan() {
                    "NaN"
                } else if x.is_sign_positive() {
                    "Infinity"
                } else {
                    "-Infinity"
                }
                .to_string(),
            )
        },
        Value::Number,
    )
}

/// A big-endian `u16` at `at`.
fn word(raw: &[u8], at: usize) -> Option<u16> {
    raw.get(at..at.checked_add(2)?)
        .and_then(|pair| <[u8; 2]>::try_from(pair).ok())
        .map(u16::from_be_bytes)
}

/// A `numeric` in its binary form — digit count, weight, sign, display
/// scale, then base-10000 digits — as the decimal PostgreSQL prints.
fn numeric(raw: &[u8]) -> std::result::Result<String, String> {
    let header = |at: usize| word(raw, at).ok_or_else(|| "a truncated numeric".to_string());
    let count = usize::from(header(0)?);
    let weight = i32::from(header(2)?.cast_signed());
    let sign = header(4)?;
    let scale = usize::from(header(6)?);
    let negative = match sign {
        0x0000 => false,
        0x4000 => true,
        0xC000 => return Ok("NaN".to_string()),
        0xD000 => return Ok("Infinity".to_string()),
        0xF000 => return Ok("-Infinity".to_string()),
        other => return Err(format!("a numeric of sign {other:#06x}")),
    };
    let digits: Vec<u16> = (0..count)
        .map(|k| word(raw, 8 + 2 * k))
        .collect::<Option<_>>()
        .ok_or_else(|| "a truncated numeric".to_string())?;
    if raw.len() != 8 + 2 * count || digits.iter().any(|d| *d > 9999) {
        return Err("a malformed numeric".to_string());
    }
    // The digit of 10000^(weight - index): zero outside those sent.
    let digit = |index: i32| {
        usize::try_from(index)
            .ok()
            .and_then(|at| digits.get(at))
            .copied()
            .unwrap_or(0)
    };
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    if weight < 0 {
        text.push('0');
    } else {
        let _ = write!(text, "{}", digit(0));
        for index in 1..=weight {
            let _ = write!(text, "{:04}", digit(index));
        }
    }
    if scale > 0 {
        // Whole groups of four past the point, cut to the scale.
        let mut fraction = String::new();
        let first = weight.saturating_add(1);
        for offset in 0..scale.div_ceil(4) {
            let offset = i32::try_from(offset).unwrap_or(i32::MAX);
            let _ = write!(fraction, "{:04}", digit(first.saturating_add(offset)));
        }
        fraction.truncate(scale);
        text.push('.');
        text.push_str(&fraction);
    }
    Ok(text)
}

/// A `uuid`'s sixteen bytes, hyphenated, lower case.
fn uuid(raw: &[u8]) -> std::result::Result<String, String> {
    let bytes =
        <[u8; 16]>::try_from(raw).map_err(|_| format!("{} byte(s) for a uuid", raw.len()))?;
    let mut text = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// A `timestamptz` — microseconds since 2000-01-01 UTC — in RFC 3339, UTC.
fn rfc3339(micros: i64) -> String {
    match micros {
        i64::MAX => return "infinity".to_string(),
        i64::MIN => return "-infinity".to_string(),
        _ => {}
    }
    let unix = i128::from(micros) + EPOCH_2000_MICROS;
    let seconds = unix.div_euclid(1_000_000);
    let fraction = unix.rem_euclid(1_000_000);
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let of_day = seconds.rem_euclid(86_400);
    let mut text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    );
    if fraction != 0 {
        let _ = write!(text, ".{fraction:06}");
    }
    text.push('Z');
    text
}

/// The proleptic Gregorian date of a day counted from 1970-01-01
/// (H. Hinnant's `civil_from_days`).
const fn civil(days: i128) -> (i128, i128, i128) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// A JSON variable, typed for the parameter it binds.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Bound {
    /// `NULL`, whatever the parameter's type.
    Null,
    /// An `int2`.
    Int2(i16),
    /// An `int4`.
    Int4(i32),
    /// An `int8`.
    Int8(i64),
    /// A `float4`.
    Float4(f32),
    /// A `float8`.
    Float8(f64),
    /// A `bool`.
    Bool(bool),
    /// Text, `varchar` and their kin.
    Text(String),
    /// `json` or `jsonb`.
    Json(Value),
    /// A `uuid`'s sixteen bytes.
    Uuid([u8; 16]),
}

impl ToSql for Bound {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> std::result::Result<IsNull, Failure> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Int2(n) => n.to_sql(ty, out),
            Self::Int4(n) => n.to_sql(ty, out),
            Self::Int8(n) => n.to_sql(ty, out),
            Self::Float4(x) => x.to_sql(ty, out),
            Self::Float8(x) => x.to_sql(ty, out),
            Self::Bool(b) => b.to_sql(ty, out),
            Self::Text(text) => text.to_sql(ty, out),
            Self::Json(json) => json.to_sql(ty, out),
            Self::Uuid(bytes) => {
                out.extend_from_slice(bytes);
                Ok(IsNull::No)
            }
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

/// The variables, each typed for the parameter of the query it binds, in
/// order.
///
/// # Errors
///
/// [`Error::Store`] when the query binds another number of parameters than
/// the variables given, or a variable does not fit its parameter's type —
/// naming the parameter. A type with no binding here (`numeric`,
/// `timestamptz`, …) is cast in the query from text: `$1::text::numeric`.
pub(super) fn bound(types: &[Type], values: &[Value]) -> Result<Vec<Bound>> {
    if types.len() != values.len() {
        return Err(Error::Store(format!(
            "the query binds {} parameter(s), the Capability lists {} variable(s)",
            types.len(),
            values.len()
        )));
    }
    types
        .iter()
        .zip(values)
        .enumerate()
        .map(|(index, (ty, value))| {
            bind(ty, value)
                .map_err(|why| Error::Store(format!("parameter ${} ({ty}): {why}", index + 1)))
        })
        .collect()
}

/// One variable typed for `ty`.
fn bind(ty: &Type, value: &Value) -> std::result::Result<Bound, String> {
    if value.is_null() {
        return Ok(Bound::Null);
    }
    let integer = || {
        value
            .as_i64()
            .ok_or_else(|| format!("{value} is not an integer"))
    };
    let narrow = |n: i64| format!("{n} is out of range");
    let bound = if *ty == Type::INT2 {
        let n = integer()?;
        Bound::Int2(i16::try_from(n).map_err(|_| narrow(n))?)
    } else if *ty == Type::INT4 {
        let n = integer()?;
        Bound::Int4(i32::try_from(n).map_err(|_| narrow(n))?)
    } else if *ty == Type::INT8 {
        Bound::Int8(integer()?)
    } else if *ty == Type::FLOAT4 || *ty == Type::FLOAT8 {
        let x = value
            .as_f64()
            .ok_or_else(|| format!("{value} is not a number"))?;
        if *ty == Type::FLOAT4 {
            // The nearest `f32` is what a `float4` column keeps anyway.
            #[allow(clippy::cast_possible_truncation)]
            Bound::Float4(x as f32)
        } else {
            Bound::Float8(x)
        }
    } else if *ty == Type::BOOL {
        Bound::Bool(
            value
                .as_bool()
                .ok_or_else(|| format!("{value} is not a boolean"))?,
        )
    } else if *ty == Type::JSON || *ty == Type::JSONB {
        Bound::Json(value.clone())
    } else if *ty == Type::UUID {
        let text = value
            .as_str()
            .ok_or_else(|| format!("{value} is not a uuid"))?;
        Bound::Uuid(uuid_bytes(text).ok_or_else(|| format!("{text} is not a uuid"))?)
    } else if <String as ToSql>::accepts(ty) {
        match value {
            Value::String(text) => Bound::Text(text.clone()),
            Value::Number(_) | Value::Bool(_) => Bound::Text(value.to_string()),
            _ => return Err(format!("{value} is not text")),
        }
    } else {
        return Err("no binding from JSON: cast it from text in the query".to_string());
    };
    Ok(bound)
}

/// A uuid's text — hyphenated or not, any case — as its sixteen bytes.
fn uuid_bytes(text: &str) -> Option<[u8; 16]> {
    let hex: Vec<u8> = text.bytes().filter(|b| *b != b'-').collect();
    if hex.len() != 32 || text.len() - hex.len() > 4 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (byte, pair) in bytes.iter_mut().zip(hex.chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    /// A `numeric` in its binary form.
    fn numeric_bytes(weight: i16, sign: u16, scale: u16, digits: &[u16]) -> Vec<u8> {
        let count = u16::try_from(digits.len()).expect("few digits");
        let mut raw = Vec::new();
        for word in [count, weight.cast_unsigned(), sign, scale] {
            raw.extend_from_slice(&word.to_be_bytes());
        }
        for digit in digits {
            raw.extend_from_slice(&digit.to_be_bytes());
        }
        raw
    }

    fn one(ty: &Type, raw: &[u8]) -> Result<Value> {
        object(&[("c", ty)], &[Some(raw)]).map(|row| row["c"].clone())
    }

    #[test]
    fn a_row_is_an_object_of_its_columns_and_null_is_null() {
        let row = object(
            &[
                ("id", &Type::INT8),
                ("name", &Type::TEXT),
                ("gone", &Type::INT4),
            ],
            &[Some(&881i64.to_be_bytes()), Some(b"Ada"), None],
        )
        .expect("mapped");
        assert_eq!(row, json!({ "id": 881, "name": "Ada", "gone": null }));
        assert_eq!(
            object(&[], &[]).expect("mapped"),
            json!({}),
            "a row of no column"
        );
        assert!(
            matches!(
                object(&[("id", &Type::INT8)], &[]),
                Err(Error::Store(why)) if why.contains("0 value(s) for 1 column(s)")
            ),
            "values and columns disagree"
        );
    }

    #[test]
    fn the_common_types_map_to_their_json_shape() {
        assert_eq!(
            one(&Type::INT2, &(-7i16).to_be_bytes()).ok(),
            Some(json!(-7))
        );
        assert_eq!(
            one(&Type::INT4, &70_000i32.to_be_bytes()).ok(),
            Some(json!(70_000))
        );
        assert_eq!(
            one(&Type::INT8, &i64::MAX.to_be_bytes()).ok(),
            Some(json!(i64::MAX))
        );
        assert_eq!(
            one(&Type::FLOAT4, &0.1f32.to_be_bytes()).ok(),
            Some(json!(0.1))
        );
        assert_eq!(
            one(&Type::FLOAT8, &2.5f64.to_be_bytes()).ok(),
            Some(json!(2.5))
        );
        assert_eq!(one(&Type::BOOL, &[1]).ok(), Some(json!(true)));
        assert_eq!(one(&Type::BOOL, &[0]).ok(), Some(json!(false)));
        assert_eq!(one(&Type::TEXT, b"text").ok(), Some(json!("text")));
        assert_eq!(one(&Type::VARCHAR, b"varchar").ok(), Some(json!("varchar")));
        assert_eq!(one(&Type::BPCHAR, b"char  ").ok(), Some(json!("char  ")));
        assert_eq!(one(&Type::NAME, b"name").ok(), Some(json!("name")));
        assert_eq!(
            one(&Type::JSON, br#"{"a":[1,2]}"#).ok(),
            Some(json!({ "a": [1, 2] }))
        );
        assert_eq!(
            one(&Type::JSONB, b"\x01{\"b\":true}").ok(),
            Some(json!({ "b": true })),
            "jsonb carries its version byte"
        );
        assert_eq!(
            one(&Type::NUMERIC, &numeric_bytes(1, 0, 3, &[1, 2345, 6780])).ok(),
            Some(json!("12345.678"))
        );
        let id = [
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ];
        assert_eq!(
            one(&Type::UUID, &id).ok(),
            Some(json!("550e8400-e29b-41d4-a716-446655440000"))
        );
        assert_eq!(
            one(&Type::TIMESTAMPTZ, &0i64.to_be_bytes()).ok(),
            Some(json!("2000-01-01T00:00:00Z"))
        );
    }

    #[test]
    fn a_non_finite_float_is_its_postgres_spelling() {
        assert_eq!(float(f64::NAN), json!("NaN"));
        assert_eq!(float(f64::INFINITY), json!("Infinity"));
        assert_eq!(float(f64::NEG_INFINITY), json!("-Infinity"));
        assert_eq!(float(-0.5), json!(-0.5));
        assert_eq!(
            one(&Type::FLOAT4, &f32::NAN.to_be_bytes()).ok(),
            Some(json!("NaN"))
        );
        assert_eq!(
            one(&Type::FLOAT4, &f32::NEG_INFINITY.to_be_bytes()).ok(),
            Some(json!("-Infinity"))
        );
    }

    #[test]
    fn an_unsupported_type_or_bad_bytes_name_the_column() {
        for (ty, raw) in [
            (Type::BYTEA, &b"\x00"[..]),
            (Type::DATE, &0i32.to_be_bytes()[..]),
        ] {
            let refused = object(&[("photo", &ty)], &[Some(raw)]);
            assert!(
                matches!(&refused, Err(Error::Store(why)) if why.starts_with("column photo: the type") && why.contains("no JSON mapping")),
                "{refused:?}"
            );
        }
        for (ty, raw) in [
            (Type::INT4, &b"\x00"[..]),
            (Type::UUID, &b"\x00\x01"[..]),
            (Type::TIMESTAMPTZ, &b"\x00"[..]),
            (Type::NUMERIC, &b"\x00"[..]),
            (Type::TEXT, &b"\xff"[..]),
        ] {
            let refused = object(&[("broken", &ty)], &[Some(raw)]);
            assert!(
                matches!(&refused, Err(Error::Store(why)) if why.starts_with("column broken: ")),
                "{ty}: {refused:?}"
            );
        }
        assert!(
            one(&Type::TIMESTAMPTZ, b"\x00")
                .is_err_and(|e| e.to_string().contains("1 byte(s) for a timestamptz"))
        );
    }

    #[test]
    fn a_numeric_prints_as_postgres_prints_it() {
        for (raw, printed) in [
            (numeric_bytes(0, 0, 0, &[]), "0"),
            (numeric_bytes(1, 0, 4, &[9999, 9999, 9999]), "99999999.9999"),
            (numeric_bytes(0, 0, 2, &[]), "0.00"),
            (numeric_bytes(0, 0x4000, 0, &[1]), "-1"),
            (numeric_bytes(2, 0, 0, &[1]), "100000000"),
            (numeric_bytes(1, 0, 0, &[12, 5]), "120005"),
            (numeric_bytes(-1, 0, 4, &[12]), "0.0012"),
            (numeric_bytes(-2, 0x4000, 6, &[500]), "-0.000005"),
            (numeric_bytes(0, 0, 5, &[3, 1415, 9000]), "3.14159"),
            (numeric_bytes(0, 0, 10, &[3, 1415]), "3.1415000000"),
            (numeric_bytes(0, 0xC000, 0, &[]), "NaN"),
            (numeric_bytes(0, 0xD000, 0, &[]), "Infinity"),
            (numeric_bytes(0, 0xF000, 0, &[]), "-Infinity"),
        ] {
            assert_eq!(numeric(&raw).as_deref(), Ok(printed), "{raw:?}");
        }
        for broken in [
            numeric_bytes(0, 0x1234, 0, &[1]),
            numeric_bytes(0, 0, 0, &[10_000]),
            [numeric_bytes(0, 0, 0, &[1]), vec![0]].concat(),
            numeric_bytes(0, 0, 0, &[1, 2])[..10].to_vec(),
            vec![0, 0, 0, 0, 0, 0, 0],
        ] {
            assert!(numeric(&broken).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn a_timestamptz_prints_in_rfc_3339_utc() {
        let since_2000 =
            |unix_seconds: i64, micros: i64| (unix_seconds - 946_684_800) * 1_000_000 + micros;
        for (micros, printed) in [
            (0, "2000-01-01T00:00:00Z"),
            (1, "2000-01-01T00:00:00.000001Z"),
            (-1, "1999-12-31T23:59:59.999999Z"),
            (since_2000(0, 0), "1970-01-01T00:00:00Z"),
            (since_2000(1_709_210_096, 0), "2024-02-29T12:34:56Z"),
            (
                since_2000(1_709_251_199, 500_000),
                "2024-02-29T23:59:59.500000Z",
            ),
            (since_2000(4_107_542_400, 0), "2100-03-01T00:00:00Z"),
            (since_2000(951_782_400, 0), "2000-02-29T00:00:00Z"),
            (i64::MAX, "infinity"),
            (i64::MIN, "-infinity"),
        ] {
            assert_eq!(rfc3339(micros), printed, "{micros}");
        }
    }

    /// The day count of a date: `civil`'s inverse (`days_from_civil`).
    const fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year.rem_euclid(400);
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    proptest! {
        #[test]
        fn a_timestamptz_reads_back_to_its_instant(micros in -200_000_000_000_000_000i64..200_000_000_000_000_000) {
            let text = rfc3339(micros);
            let (date, time) = text.trim_end_matches('Z').split_once('T').expect("a T");
            let mut date = date.rsplitn(3, '-');
            let day: i128 = date.next().expect("day").parse().expect("day");
            let month: i128 = date.next().expect("month").parse().expect("month");
            let year: i128 = date.next().expect("year").parse().expect("year");
            prop_assert!((1..=12).contains(&month) && (1..=31).contains(&day), "{}", text);
            let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
            let parts: Vec<i128> = clock.split(':').map(|p| p.parse().expect("a number")).collect();
            prop_assert_eq!(parts.len(), 3);
            let seconds = days_from_civil(year, month, day) * 86_400 + parts[0] * 3600 + parts[1] * 60 + parts[2];
            let fraction: i128 = format!("{fraction:0<6}").parse().expect("micros");
            prop_assert_eq!(seconds * 1_000_000 + fraction, i128::from(micros) + EPOCH_2000_MICROS, "{}", text);
        }

        #[test]
        fn a_numeric_reads_as_its_decimal(integer in 0u64..u64::MAX, fraction in "[0-9]{0,12}", negative in any::<bool>()) {
            // The decimal's digits, grouped by four around the point.
            let whole = integer.to_string();
            let pad = (4 - whole.len() % 4) % 4;
            let whole_digits = format!("{}{whole}", "0".repeat(pad));
            let fraction_digits = format!("{fraction}{}", "0".repeat((4 - fraction.len() % 4) % 4));
            let groups: Vec<u16> = whole_digits
                .as_bytes()
                .chunks(4)
                .chain(fraction_digits.as_bytes().chunks(4))
                .map(|g| std::str::from_utf8(g).expect("ascii").parse().expect("a group"))
                .collect();
            let weight = i16::try_from(whole_digits.len() / 4).expect("small") - 1;
            let raw = numeric_bytes(weight, if negative { 0x4000 } else { 0 }, u16::try_from(fraction.len()).expect("small"), &groups);
            let expected = format!(
                "{}{integer}{}{fraction}",
                if negative { "-" } else { "" },
                if fraction.is_empty() { "" } else { "." }
            );
            prop_assert_eq!(numeric(&raw), Ok(expected));
        }
    }

    #[test]
    fn raw_takes_any_type_and_null() {
        assert!(<Raw<'_> as FromSql<'_>>::accepts(&Type::BYTEA));
        let some = Raw::from_sql(&Type::INT4, b"abcd").expect("bytes");
        assert_eq!(some.0, Some(&b"abcd"[..]));
        let none = Raw::from_sql_null(&Type::INT4).expect("null");
        assert_eq!(none.0, None);
    }

    #[test]
    fn variables_are_typed_by_their_parameter_in_order() {
        let types = [
            Type::INT2,
            Type::INT4,
            Type::INT8,
            Type::FLOAT4,
            Type::FLOAT8,
            Type::BOOL,
            Type::TEXT,
            Type::VARCHAR,
            Type::JSONB,
            Type::UUID,
            Type::NUMERIC,
        ];
        let values = [
            json!(-2),
            json!(40_000),
            json!(1_i64 << 40),
            json!(0.5),
            json!(2.25),
            json!(true),
            json!("881"),
            json!(881),
            json!({ "a": 1 }),
            json!("550E8400-e29b-41d4-a716-446655440000"),
            Value::Null,
        ];
        assert_eq!(
            bound(&types, &values).expect("bound"),
            [
                Bound::Int2(-2),
                Bound::Int4(40_000),
                Bound::Int8(1 << 40),
                Bound::Float4(0.5),
                Bound::Float8(2.25),
                Bound::Bool(true),
                Bound::Text("881".to_string()),
                Bound::Text("881".to_string()),
                Bound::Json(json!({ "a": 1 })),
                Bound::Uuid([
                    0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55,
                    0x44, 0x00, 0x00
                ]),
                Bound::Null,
            ]
        );
        assert_eq!(
            bound(&[Type::TEXT], &[json!(false)]).expect("bound"),
            [Bound::Text("false".to_string())]
        );
        assert_eq!(
            bound(&[Type::JSON], &[json!([1])]).expect("bound"),
            [Bound::Json(json!([1]))]
        );
        assert_eq!(
            bound(&[Type::FLOAT8], &[json!(3)]).expect("bound"),
            [Bound::Float8(3.0)],
            "an integer is a number"
        );
        assert_eq!(bound(&[], &[]).expect("bound"), []);
    }

    #[test]
    fn a_variable_that_does_not_fit_names_its_parameter() {
        for (ty, value, why) in [
            (Type::INT2, json!(40_000), "40000 is out of range"),
            (Type::INT4, json!(1_i64 << 40), "is out of range"),
            (Type::INT8, json!("12"), "is not an integer"),
            (Type::INT4, json!(1.5), "is not an integer"),
            (Type::FLOAT8, json!("1.5"), "is not a number"),
            (Type::FLOAT4, json!(true), "is not a number"),
            (Type::BOOL, json!(1), "is not a boolean"),
            (Type::TEXT, json!([1]), "is not text"),
            (Type::TEXT, json!({ "a": 1 }), "is not text"),
            (Type::UUID, json!(12), "is not a uuid"),
            (Type::UUID, json!("550e8400"), "is not a uuid"),
            (Type::NUMERIC, json!("1.5"), "cast it from text"),
            (
                Type::TIMESTAMPTZ,
                json!("2026-01-01T00:00:00Z"),
                "cast it from text",
            ),
        ] {
            let refused = bound(&[Type::TEXT, ty.clone()], &[json!("x"), value]);
            assert!(
                matches!(&refused, Err(Error::Store(m)) if m.starts_with(&format!("parameter $2 ({ty}): ")) && m.contains(why)),
                "{ty}: {refused:?}"
            );
        }
        let counted = bound(&[Type::TEXT], &[json!("a"), json!("b")]);
        assert!(
            matches!(&counted, Err(Error::Store(m)) if m.contains("binds 1 parameter(s), the Capability lists 2")),
            "{counted:?}"
        );
    }

    #[test]
    fn a_uuid_reads_hyphenated_or_not_and_nothing_else() {
        let id = [
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ];
        assert_eq!(uuid_bytes("550e8400-e29b-41d4-a716-446655440000"), Some(id));
        assert_eq!(uuid_bytes("550e8400e29b41d4a716446655440000"), Some(id));
        assert_eq!(
            uuid_bytes("550e8400-e29b-41d4-a716-44665544000"),
            None,
            "short"
        );
        assert_eq!(
            uuid_bytes("550e8400-e29b-41d4-a716-4466554400zz"),
            None,
            "not hex"
        );
        assert_eq!(
            uuid_bytes("550e8400--e29b-41d4-a716-446655440000-"),
            None,
            "too many hyphens"
        );
        assert_eq!(
            uuid(&id).as_deref(),
            Ok("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn a_bound_variable_encodes_as_its_type_does() {
        let encoded = |bound: &Bound, ty: &Type| {
            let mut out = BytesMut::new();
            let null = bound.to_sql_checked(ty, &mut out).expect("encoded");
            (matches!(null, IsNull::Yes), out.to_vec())
        };
        assert_eq!(encoded(&Bound::Null, &Type::INT4), (true, Vec::new()));
        assert_eq!(
            encoded(&Bound::Int2(-2), &Type::INT2),
            (false, (-2i16).to_be_bytes().to_vec())
        );
        assert_eq!(
            encoded(&Bound::Int4(7), &Type::INT4),
            (false, 7i32.to_be_bytes().to_vec())
        );
        assert_eq!(
            encoded(&Bound::Int8(9), &Type::INT8),
            (false, 9i64.to_be_bytes().to_vec())
        );
        assert_eq!(
            encoded(&Bound::Float4(0.5), &Type::FLOAT4),
            (false, 0.5f32.to_be_bytes().to_vec())
        );
        assert_eq!(
            encoded(&Bound::Float8(0.25), &Type::FLOAT8),
            (false, 0.25f64.to_be_bytes().to_vec())
        );
        assert_eq!(encoded(&Bound::Bool(true), &Type::BOOL), (false, vec![1]));
        assert_eq!(
            encoded(&Bound::Text("hé".to_string()), &Type::TEXT),
            (false, "hé".as_bytes().to_vec())
        );
        assert_eq!(
            encoded(&Bound::Json(json!({ "a": 1 })), &Type::JSONB),
            (false, b"\x01{\"a\":1}".to_vec())
        );
        assert_eq!(
            encoded(&Bound::Uuid([7; 16]), &Type::UUID),
            (false, vec![7; 16])
        );
        assert!(<Bound as ToSql>::accepts(&Type::BYTEA));
    }
}

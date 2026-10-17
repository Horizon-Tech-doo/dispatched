//! Typed custom property values, durations and times as users type them.

use super::message::{PropValue, TIMESPAN_DESCRIPTOR};
use crate::{Error, Result};
use fe2o3_amqp::types::primitives::{SimpleValue, Timestamp, Value};
use serde_amqp::described::Described;
use serde_amqp::descriptor::Descriptor;
use serde_amqp::primitives::Symbol;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const TICKS_PER_SECOND: i64 = 10_000_000;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// The type of a custom property as named by users.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    /// Text.
    String,
    /// 32-bit signed integer.
    Int,
    /// 64-bit signed integer.
    Long,
    /// 64-bit float.
    Double,
    /// true or false.
    Bool,
    /// A point in time (AMQP timestamp).
    DateTime,
    /// A duration (.NET `TimeSpan`).
    TimeSpan,
    /// A GUID (AMQP uuid).
    Guid,
}

impl PropType {
    /// Parse a short (`int`) or .NET (`Int32`) type name, ignoring case.
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "string" => PropType::String,
            "int" | "int32" => PropType::Int,
            "long" | "int64" => PropType::Long,
            "double" => PropType::Double,
            "bool" | "boolean" => PropType::Bool,
            "datetime" => PropType::DateTime,
            "timespan" => PropType::TimeSpan,
            "guid" => PropType::Guid,
            _ => {
                return Err(Error::Invalid(format!(
                    "unknown property type '{name}'; use string, int, long, double, bool, datetime, timespan or guid"
                )));
            }
        })
    }

    /// The short lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            PropType::String => "string",
            PropType::Int => "int",
            PropType::Long => "long",
            PropType::Double => "double",
            PropType::Bool => "bool",
            PropType::DateTime => "datetime",
            PropType::TimeSpan => "timespan",
            PropType::Guid => "guid",
        }
    }
}

/// A custom property value with the type it will be sent as.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedValue {
    /// Text.
    String(String),
    /// 32-bit signed integer.
    Int(i32),
    /// 64-bit signed integer.
    Long(i64),
    /// 64-bit float.
    Double(f64),
    /// true or false.
    Bool(bool),
    /// A point in time.
    DateTime(OffsetDateTime),
    /// A duration.
    TimeSpan(time::Duration),
    /// A GUID.
    Guid(uuid::Uuid),
}

impl TypedValue {
    /// Parse `text` as the given type.
    pub fn parse(ty: PropType, text: &str) -> Result<Self> {
        let bad = |why: &str| Error::Invalid(format!("'{text}' is not a valid {}{why}", ty.name()));
        Ok(match ty {
            PropType::String => TypedValue::String(text.to_string()),
            PropType::Int => match text.trim().parse::<i32>() {
                Ok(n) => TypedValue::Int(n),
                Err(_) if text.trim().parse::<i64>().is_ok() => {
                    return Err(bad("; it does not fit 32 bits, use :long"));
                }
                Err(_) => return Err(bad("")),
            },
            PropType::Long => TypedValue::Long(text.trim().parse().map_err(|_| bad(""))?),
            PropType::Double => {
                let d: f64 = text.trim().parse().map_err(|_| bad(""))?;
                if !d.is_finite() {
                    return Err(bad(""));
                }
                TypedValue::Double(d)
            }
            PropType::Bool => match text.trim().to_ascii_lowercase().as_str() {
                "true" => TypedValue::Bool(true),
                "false" => TypedValue::Bool(false),
                _ => return Err(bad("; use true or false")),
            },
            PropType::DateTime => TypedValue::DateTime(
                OffsetDateTime::parse(text.trim(), &Rfc3339)
                    .map_err(|_| bad("; use an RFC 3339 time, such as 2026-10-17T12:00:00Z"))?,
            ),
            PropType::TimeSpan => TypedValue::TimeSpan(
                parse_span(text).map_err(|_| bad("; use 1h30m, PT1H30M or [d.]hh:mm:ss"))?,
            ),
            PropType::Guid => {
                TypedValue::Guid(uuid::Uuid::parse_str(text.trim()).map_err(|_| bad(""))?)
            }
        })
    }

    /// The AMQP value, encoded as the .NET SDK does.
    pub fn to_simple(&self) -> SimpleValue {
        match self {
            TypedValue::String(s) => SimpleValue::String(s.clone()),
            TypedValue::Int(n) => SimpleValue::Int(*n),
            TypedValue::Long(n) => SimpleValue::Long(*n),
            TypedValue::Double(d) => SimpleValue::Double((*d).into()),
            TypedValue::Bool(b) => SimpleValue::Bool(*b),
            TypedValue::DateTime(t) => SimpleValue::Timestamp(Timestamp::from(
                i64::try_from(t.unix_timestamp_nanos().div_euclid(1_000_000)).unwrap_or(i64::MAX),
            )),
            TypedValue::TimeSpan(d) => SimpleValue::Described(Box::new(Described {
                descriptor: Descriptor::Name(Symbol::from(TIMESPAN_DESCRIPTOR)),
                value: Value::Long(ticks_from_duration(*d)),
            })),
            TypedValue::Guid(u) => SimpleValue::Uuid((*u).into()),
        }
    }

    /// The one conversion used by cloning, import and templates; `None` for values that cannot be
    /// copied (binary, other, null), which the caller reports.
    pub fn from_prop(p: &PropValue) -> Option<Self> {
        Some(match p {
            PropValue::String(s) => TypedValue::String(s.clone()),
            PropValue::Bool(b) => TypedValue::Bool(*b),
            PropValue::Double(d) => TypedValue::Double(*d),
            PropValue::Int(n) => match i32::try_from(*n) {
                Ok(n) => TypedValue::Int(n),
                Err(_) => TypedValue::Long(*n),
            },
            PropValue::UInt(n) => TypedValue::Long(i64::try_from(*n).ok()?),
            PropValue::Timestamp(t) => TypedValue::DateTime(*t),
            PropValue::TimeSpan(d) => TypedValue::TimeSpan(*d),
            PropValue::Uuid(u) => TypedValue::Guid(*u),
            PropValue::Binary(_) | PropValue::Other(_) | PropValue::Null => return None,
        })
    }
}

/// Parse `KEY=VALUE[:TYPE]`: split on the first `=`, then on the last `:` only when the suffix
/// names a type.
pub fn parse_prop_arg(arg: &str) -> Result<(String, TypedValue)> {
    let (key, rest) = arg.split_once('=').ok_or_else(|| {
        Error::Invalid(format!("property '{arg}' needs the form KEY=VALUE[:TYPE]"))
    })?;
    if key.is_empty() {
        return Err(Error::Invalid(format!(
            "property '{arg}' has no name; use KEY=VALUE[:TYPE]"
        )));
    }
    let (text, ty) = match rest.rsplit_once(':') {
        Some((text, suffix)) => match PropType::parse(suffix) {
            Ok(ty) => (text, ty),
            Err(_) => (rest, PropType::String),
        },
        None => (rest, PropType::String),
    };
    let value = TypedValue::parse(ty, text)
        .map_err(|e| Error::Invalid(format!("property '{key}': {e}")))?;
    Ok((key.to_string(), value))
}

/// .NET ticks (100 ns) to a duration.
pub(crate) fn duration_from_ticks(ticks: i64) -> time::Duration {
    time::Duration::new(
        ticks / TICKS_PER_SECOND,
        ((ticks % TICKS_PER_SECOND) * 100) as i32,
    )
}

fn ticks_from_duration(d: time::Duration) -> i64 {
    i64::try_from(d.whole_nanoseconds() / 100).unwrap_or(if d.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

/// ISO 8601 duration text for a span, such as `PT1H30M`, `P1DT2H`, `PT1.5S`, `-PT5M`.
pub(crate) fn format_iso_duration(d: time::Duration) -> String {
    let neg = d.is_negative();
    let total = d.unsigned_abs();
    let secs = total.as_secs();
    let nanos = total.subsec_nanos();
    let (days, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let mut out = String::from(if neg { "-P" } else { "P" });
    if days > 0 {
        out.push_str(&format!("{days}D"));
    }
    if h > 0 || m > 0 || s > 0 || nanos > 0 || days == 0 {
        out.push('T');
        if h > 0 {
            out.push_str(&format!("{h}H"));
        }
        if m > 0 {
            out.push_str(&format!("{m}M"));
        }
        if s > 0 || nanos > 0 || (h == 0 && m == 0 && days == 0) {
            if nanos > 0 {
                let frac = format!("{nanos:09}");
                out.push_str(&format!("{s}.{}S", frac.trim_end_matches('0')));
            } else {
                out.push_str(&format!("{s}S"));
            }
        }
    }
    out
}

/// Signed total nanoseconds of a duration text, in any documented form.
fn parse_nanos(text: &str) -> Result<i128> {
    let bad = || {
        Error::Invalid(format!(
            "'{text}' is not a duration; use 90s, 15m, 2h, 7d, 1h30m, PT1H30M or [d.]hh:mm:ss"
        ))
    };
    let t = text.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if t.is_empty() {
        return Err(bad());
    }
    let n = if t.starts_with(['P', 'p']) {
        parse_iso(&t[1..]).ok_or_else(bad)?
    } else if t.contains(':') {
        parse_clock(t).ok_or_else(bad)?
    } else {
        parse_units(t).ok_or_else(bad)?
    };
    Ok(if neg { -(n as i128) } else { n as i128 })
}

/// Digits with an optional fraction to nanoseconds scaled by `unit` nanoseconds.
fn decimal_nanos(num: &str, unit: u128) -> Option<u128> {
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    if whole.is_empty() && frac.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac: String = frac.chars().take(9).collect();
    let f: u128 = if frac.is_empty() {
        0
    } else {
        frac.parse::<u128>().ok()? * 10u128.pow(9 - frac.len() as u32)
    };
    w.checked_mul(unit)?
        .checked_add(f * unit / NANOS_PER_SECOND)
}

fn parse_units(t: &str) -> Option<u128> {
    let mut total: u128 = 0;
    let mut rest = t;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
        let (num, tail) = rest.split_at(end);
        let unit = match tail.chars().next()?.to_ascii_lowercase() {
            'd' => 86_400,
            'h' => 3_600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        total = total.checked_add(decimal_nanos(num, unit * NANOS_PER_SECOND)?)?;
        rest = &tail[1..];
    }
    Some(total)
}

fn parse_iso(t: &str) -> Option<u128> {
    let upper = t.to_ascii_uppercase();
    let (date, time) = match upper.split_once('T') {
        Some((d, t)) => (d.to_string(), Some(t.to_string())),
        None => (upper, None),
    };
    if date.is_empty() && time.as_deref().is_none_or(str::is_empty) {
        return None;
    }
    let mut total: u128 = 0;
    for (part, units) in [
        (date.as_str(), &[('W', 7 * 86_400u128), ('D', 86_400)][..]),
        (
            time.as_deref().unwrap_or(""),
            &[('H', 3_600), ('M', 60), ('S', 1)][..],
        ),
    ] {
        let mut rest = part;
        let mut last = 0usize;
        while !rest.is_empty() {
            let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
            let (num, tail) = rest.split_at(end);
            let designator = tail.chars().next()?;
            let idx = units.iter().position(|(c, _)| *c == designator)?;
            if idx < last {
                return None;
            }
            last = idx + 1;
            total = total.checked_add(decimal_nanos(num, units[idx].1 * NANOS_PER_SECOND)?)?;
            rest = &tail[1..];
        }
    }
    Some(total)
}

fn parse_clock(t: &str) -> Option<u128> {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let (days, hours) = match parts[0].split_once('.') {
        Some((d, h)) => (d.parse::<u128>().ok()?, h),
        None => (0, parts[0]),
    };
    let hours: u128 = hours.parse().ok()?;
    let minutes: u128 = parts[1].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let seconds = decimal_nanos(parts[2], NANOS_PER_SECOND)?;
    if seconds >= 60 * NANOS_PER_SECOND {
        return None;
    }
    days.checked_mul(86_400 * NANOS_PER_SECOND)?
        .checked_add((hours * 3600 + minutes * 60) * NANOS_PER_SECOND)?
        .checked_add(seconds)
}

/// A duration text as a signed span (zero and negative allowed).
pub(crate) fn parse_span(text: &str) -> Result<time::Duration> {
    let n = parse_nanos(text)?;
    let secs = i64::try_from(n / NANOS_PER_SECOND as i128)
        .map_err(|_| Error::Invalid(format!("'{text}' is too long a duration")))?;
    Ok(time::Duration::new(
        secs,
        (n % NANOS_PER_SECOND as i128) as i32,
    ))
}

/// Parse a positive duration: `90s`, `15m`, `2h`, `7d`, `1h30m`, ISO 8601 (`PT1H30M`, `P7D`) or
/// .NET `[d.]hh:mm:ss[.fffffff]`. Zero and negative are refused.
pub fn parse_duration(text: &str) -> Result<std::time::Duration> {
    let n = parse_nanos(text)?;
    if n <= 0 {
        return Err(Error::Invalid(format!(
            "'{text}' is not a positive duration; use 90s, 15m, 2h or 7d"
        )));
    }
    let secs = u64::try_from(n / NANOS_PER_SECOND as i128)
        .map_err(|_| Error::Invalid(format!("'{text}' is too long a duration")))?;
    Ok(std::time::Duration::new(
        secs,
        (n % NANOS_PER_SECOND as i128) as u32,
    ))
}

/// Parse a time as RFC 3339, or `+DURATION` relative to `now`.
pub fn parse_when(text: &str, now: OffsetDateTime) -> Result<OffsetDateTime> {
    let t = text.trim();
    let bad = || {
        Error::Invalid(format!(
            "'{text}' is not a time; use +DURATION, such as +10m, or an RFC 3339 time"
        ))
    };
    if let Some(rel) = t.strip_prefix('+') {
        let d = parse_duration(rel).map_err(|_| bad())?;
        return time::Duration::try_from(d)
            .ok()
            .and_then(|d| now.checked_add(d))
            .ok_or_else(bad);
    }
    OffsetDateTime::parse(t, &Rfc3339).map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn prop_args_parse_with_types_and_defaults() {
        assert_eq!(
            parse_prop_arg("kind=x").unwrap(),
            ("kind".into(), TypedValue::String("x".into()))
        );
        assert_eq!(parse_prop_arg("n=42:int").unwrap().1, TypedValue::Int(42));
        assert_eq!(
            parse_prop_arg("n=42:Int64").unwrap().1,
            TypedValue::Long(42)
        );
        assert_eq!(
            parse_prop_arg("url=http://x:string").unwrap().1,
            TypedValue::String("http://x".into())
        );
        assert_eq!(
            parse_prop_arg("at=10:30").unwrap().1,
            TypedValue::String("10:30".into()),
            "`30` is not a type"
        );
        assert_eq!(
            parse_prop_arg("w=01:30:00:timespan").unwrap().1,
            TypedValue::TimeSpan(time::Duration::minutes(90))
        );
        assert_eq!(
            parse_prop_arg("ok=TRUE:bool").unwrap().1,
            TypedValue::Bool(true)
        );
        assert!(
            matches!(parse_prop_arg("g=nope:guid"), Err(Error::Invalid(m)) if m.contains("guid"))
        );
        assert!(parse_prop_arg("=1").is_err() && parse_prop_arg("novalue").is_err());
        assert!(
            matches!(parse_prop_arg("n=3000000000:int"), Err(Error::Invalid(m)) if m.contains("long")),
            "suggests long"
        );
    }

    #[test]
    fn typed_values_encode_as_the_dotnet_sdk_does() {
        let SimpleValue::Described(d) =
            TypedValue::TimeSpan(time::Duration::seconds(1)).to_simple()
        else {
            panic!()
        };
        assert_eq!(
            d.descriptor,
            Descriptor::Name(Symbol::from("com.microsoft:timespan"))
        );
        assert_eq!(d.value, Value::Long(10_000_000));
        assert!(matches!(
            TypedValue::Guid(uuid::Uuid::nil()).to_simple(),
            SimpleValue::Uuid(_)
        ));
        assert!(matches!(
            TypedValue::DateTime(datetime!(2026-01-01 0:00 UTC)).to_simple(),
            SimpleValue::Timestamp(_)
        ));
    }

    #[test]
    fn from_prop_maps_every_copyable_value() {
        let at = datetime!(2026-01-01 0:00 UTC);
        for (p, t) in [
            (
                PropValue::String("s".into()),
                Some(TypedValue::String("s".into())),
            ),
            (PropValue::Bool(true), Some(TypedValue::Bool(true))),
            (PropValue::Double(1.5), Some(TypedValue::Double(1.5))),
            (PropValue::Int(7), Some(TypedValue::Int(7))),
            (
                PropValue::Int(3_000_000_000),
                Some(TypedValue::Long(3_000_000_000)),
            ),
            (PropValue::UInt(5), Some(TypedValue::Long(5))),
            (PropValue::UInt(u64::MAX), None),
            (PropValue::Timestamp(at), Some(TypedValue::DateTime(at))),
            (
                PropValue::TimeSpan(time::Duration::minutes(5)),
                Some(TypedValue::TimeSpan(time::Duration::minutes(5))),
            ),
            (
                PropValue::Uuid(uuid::Uuid::nil()),
                Some(TypedValue::Guid(uuid::Uuid::nil())),
            ),
            (PropValue::Binary(vec![1]), None),
            (PropValue::Null, None),
        ] {
            assert_eq!(TypedValue::from_prop(&p), t, "{p:?}");
        }
    }

    #[test]
    fn durations_and_times_parse_in_every_documented_form() {
        for (t, s) in [
            ("90s", 90),
            ("15m", 900),
            ("2h", 7200),
            ("7d", 604_800),
            ("1h30m", 5400),
            ("PT1H30M", 5400),
            ("P7D", 604_800),
            ("1.02:03:04", 93_784),
            ("00:00:01.5000000", 1),
        ] {
            assert_eq!(parse_duration(t).unwrap().as_secs(), s, "{t}");
        }
        assert!(
            parse_duration("0s").is_err()
                && parse_duration("-1m").is_err()
                && parse_duration("soon").is_err()
        );
        let now = datetime!(2026-10-17 12:00 UTC);
        assert_eq!(
            parse_when("+10m", now).unwrap(),
            datetime!(2026-10-17 12:10 UTC)
        );
        assert_eq!(
            parse_when("2026-10-18T09:00:00Z", now).unwrap(),
            datetime!(2026-10-18 9:00 UTC)
        );
    }

    #[test]
    fn malformed_times_name_both_forms() {
        let now = datetime!(2026-10-17 12:00 UTC);
        for t in ["tomorrow", "+soon", "+0s", ""] {
            assert!(
                matches!(parse_when(t, now), Err(Error::Invalid(m)) if m.contains("+DURATION") && m.contains("RFC 3339")),
                "{t}"
            );
        }
    }

    #[test]
    fn iso_text_and_ticks_round_trip() {
        for (d, s) in [
            (time::Duration::hours(1), "PT1H"),
            (time::Duration::minutes(90), "PT1H30M"),
            (time::Duration::days(7), "P7D"),
            (time::Duration::seconds(93_784), "P1DT2H3M4S"),
            (time::Duration::milliseconds(1500), "PT1.5S"),
            (time::Duration::ZERO, "PT0S"),
            (time::Duration::minutes(-5), "-PT5M"),
        ] {
            assert_eq!(format_iso_duration(d), s);
            assert_eq!(parse_span(s).unwrap(), d, "{s}");
        }
        assert_eq!(
            duration_from_ticks(15_000_000),
            time::Duration::milliseconds(1500)
        );
    }
}

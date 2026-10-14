//! Parser and evaluator for property filters, e.g.
//! `sys.DeliveryCount > 3 AND city = 'Pisa'`.

use super::lexer::{Tok, Token, err, lex};
use crate::Result;
use crate::messaging::{PeekedMessage, PropValue};
use base64::Engine;
use std::cmp::Ordering;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SysField {
    MessageId,
    CorrelationId,
    SessionId,
    Subject,
    ContentType,
    To,
    ReplyTo,
    SequenceNumber,
    EnqueuedTime,
    ScheduledEnqueueTime,
    DeliveryCount,
    DeadLetterReason,
    DeadLetterErrorDescription,
    DeadLetterSource,
    State,
    Size,
}

const SYS_NAMES: &[(&str, SysField)] = &[
    ("messageid", SysField::MessageId),
    ("correlationid", SysField::CorrelationId),
    ("sessionid", SysField::SessionId),
    ("subject", SysField::Subject),
    ("label", SysField::Subject),
    ("contenttype", SysField::ContentType),
    ("to", SysField::To),
    ("replyto", SysField::ReplyTo),
    ("sequencenumber", SysField::SequenceNumber),
    ("enqueuedtime", SysField::EnqueuedTime),
    ("scheduledenqueuetime", SysField::ScheduledEnqueueTime),
    ("deliverycount", SysField::DeliveryCount),
    ("deadletterreason", SysField::DeadLetterReason),
    (
        "deadlettererrordescription",
        SysField::DeadLetterErrorDescription,
    ),
    ("deadlettersource", SysField::DeadLetterSource),
    ("state", SysField::State),
    ("size", SysField::Size),
];

#[derive(Debug, Clone, PartialEq)]
enum Field {
    Sys(SysField),
    User(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Lit {
    /// `time` is the literal parsed as RFC 3339 once, at parse time.
    Str {
        text: String,
        time: Option<OffsetDateTime>,
    },
    Num(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cmp(Field, CmpOp, Lit),
    IsNull(Field, bool),
    /// Pattern characters; `%` and `_` are wildcards, the rest compare case-insensitively.
    Like(Field, Vec<char>, bool),
    In(Field, Vec<Lit>, bool),
}

/// A parsed property filter.
#[derive(Debug, Clone)]
pub struct PropertyFilter {
    expr: Expr,
    source: String,
}

impl PropertyFilter {
    /// Parse `src`. Errors name the column of the problem.
    pub fn parse(src: &str) -> Result<Self> {
        let tokens = lex(src)?;
        if tokens.is_empty() {
            return Err(err(1, "empty filter"));
        }
        if tokens.len() > MAX_TOKENS {
            return Err(err(
                tokens[MAX_TOKENS].col,
                format!("filter is too long (more than {MAX_TOKENS} tokens)"),
            ));
        }
        let mut p = Parser {
            tokens,
            pos: 0,
            depth: 0,
            end_col: src.chars().count() + 1,
        };
        let expr = p.or()?;
        if let Some(t) = p.peek() {
            return Err(err(t.col, format!("unexpected `{}`", describe(&t.tok))));
        }
        Ok(Self {
            expr,
            source: src.to_string(),
        })
    }

    /// The original filter text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether `msg` satisfies the filter.
    pub fn matches(&self, msg: &PeekedMessage) -> bool {
        eval(&self.expr, msg)
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.escape_debug().to_string(),
        Tok::Str(s) => format!("'{}'", s.escape_debug()),
        Tok::Num(n) => n.to_string(),
        Tok::Op(o) => (*o).to_string(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::Comma => ",".into(),
    }
}

/// Bounds that keep the recursive parser and evaluator within stack limits.
const MAX_TOKENS: usize = 4096;
const MAX_DEPTH: usize = 64;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    end_col: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn col(&self) -> usize {
        self.peek().map(|t| t.col).unwrap_or(self.end_col)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn keyword(&mut self, kw: &str) -> bool {
        if matches!(self.peek(), Some(Token { tok: Tok::Ident(s), .. }) if s.eq_ignore_ascii_case(kw))
        {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr> {
        let mut left = self.not()?;
        while self.keyword("AND") {
            left = Expr::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr> {
        if self.keyword("NOT") {
            self.enter()?;
            let inner = self.not();
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(inner?)));
        }
        self.primary()
    }

    fn enter(&mut self) -> Result<()> {
        if self.depth >= MAX_DEPTH {
            // The token at `pos - 1` was just consumed by the caller.
            let col = self
                .tokens
                .get(self.pos.saturating_sub(1))
                .map_or(self.end_col, |t| t.col);
            return Err(err(
                col,
                format!("filter is too deeply nested (more than {MAX_DEPTH} levels)"),
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn primary(&mut self) -> Result<Expr> {
        if matches!(
            self.peek(),
            Some(Token {
                tok: Tok::LParen,
                ..
            })
        ) {
            self.pos += 1;
            self.enter()?;
            let e = self.or();
            self.depth -= 1;
            let e = e?;
            let col = self.col();
            return match self.next() {
                Some(Token {
                    tok: Tok::RParen, ..
                }) => Ok(e),
                _ => Err(err(col, "expected `)`")),
            };
        }
        let field = self.field()?;
        if self.keyword("IS") {
            let negated = self.keyword("NOT");
            if !self.keyword("NULL") {
                return Err(err(self.col(), "expected NULL"));
            }
            return Ok(Expr::IsNull(field, negated));
        }
        let negated = self.keyword("NOT");
        if self.keyword("LIKE") {
            let col = self.col();
            return match self.next() {
                Some(Token {
                    tok: Tok::Str(p), ..
                }) => Ok(Expr::Like(field, p.chars().collect(), negated)),
                _ => Err(err(col, "LIKE needs a 'string' pattern")),
            };
        }
        if self.keyword("IN") {
            let col = self.col();
            match self.next() {
                Some(Token {
                    tok: Tok::LParen, ..
                }) => {}
                _ => return Err(err(col, "IN needs a parenthesised list")),
            }
            let mut values = vec![self.literal()?];
            loop {
                let col = self.col();
                match self.next() {
                    Some(Token {
                        tok: Tok::Comma, ..
                    }) => values.push(self.literal()?),
                    Some(Token {
                        tok: Tok::RParen, ..
                    }) => break,
                    _ => return Err(err(col, "expected `,` or `)`")),
                }
            }
            return Ok(Expr::In(field, values, negated));
        }
        if negated {
            return Err(err(self.col(), "expected LIKE or IN after NOT"));
        }
        let op = match self.next() {
            Some(Token {
                tok: Tok::Op(o), ..
            }) => match o {
                "=" => CmpOp::Eq,
                "!=" => CmpOp::Ne,
                "<" => CmpOp::Lt,
                "<=" => CmpOp::Le,
                ">" => CmpOp::Gt,
                _ => CmpOp::Ge,
            },
            Some(t) => {
                return Err(err(
                    t.col,
                    format!(
                        "expected a comparison operator, found `{}`",
                        describe(&t.tok)
                    ),
                ));
            }
            None => return Err(err(self.end_col, "expected a comparison operator")),
        };
        Ok(Expr::Cmp(field, op, self.literal()?))
    }

    fn field(&mut self) -> Result<Field> {
        match self.next() {
            Some(Token {
                tok: Tok::Ident(name),
                col,
            }) => {
                let lower = name.to_ascii_lowercase();
                let shown = name.escape_debug();
                if lower == "sys." || lower == "user." {
                    Err(err(col, format!("missing property name after `{shown}`")))
                } else if let Some(rest) = lower.strip_prefix("sys.") {
                    SYS_NAMES
                        .iter()
                        .find(|(n, _)| *n == rest)
                        .map(|(_, f)| Field::Sys(*f))
                        .ok_or_else(|| err(col, format!("unknown system property `{shown}` (known: MessageId, CorrelationId, SessionId, Subject, ContentType, To, ReplyTo, SequenceNumber, EnqueuedTime, ScheduledEnqueueTime, DeliveryCount, DeadLetterReason, DeadLetterErrorDescription, DeadLetterSource, State, Size)")))
                } else if lower.starts_with("user.") {
                    Ok(Field::User(name[5..].to_string()))
                } else {
                    Ok(Field::User(name))
                }
            }
            Some(t) => Err(err(
                t.col,
                format!("expected a property name, found `{}`", describe(&t.tok)),
            )),
            None => Err(err(self.end_col, "expected a property name")),
        }
    }

    fn literal(&mut self) -> Result<Lit> {
        match self.next() {
            Some(Token {
                tok: Tok::Str(s), ..
            }) => {
                let time = OffsetDateTime::parse(&s, &Rfc3339).ok();
                Ok(Lit::Str { text: s, time })
            }
            Some(Token {
                tok: Tok::Num(n), ..
            }) => Ok(Lit::Num(n)),
            Some(Token {
                tok: Tok::Ident(s), ..
            }) if s.eq_ignore_ascii_case("true") => Ok(Lit::Bool(true)),
            Some(Token {
                tok: Tok::Ident(s), ..
            }) if s.eq_ignore_ascii_case("false") => Ok(Lit::Bool(false)),
            Some(t) => Err(err(
                t.col,
                format!("expected a value, found `{}`", describe(&t.tok)),
            )),
            None => Err(err(self.end_col, "expected a value")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum FieldValue {
    Null,
    Str(String),
    Num(f64),
    Bool(bool),
    Time(OffsetDateTime),
}

fn opt_str(v: &Option<String>) -> FieldValue {
    v.clone().map(FieldValue::Str).unwrap_or(FieldValue::Null)
}

fn field_value(msg: &PeekedMessage, field: &Field) -> FieldValue {
    match field {
        Field::Sys(f) => match f {
            SysField::MessageId => opt_str(&msg.message_id),
            SysField::CorrelationId => opt_str(&msg.correlation_id),
            SysField::SessionId => opt_str(&msg.session_id),
            SysField::Subject => opt_str(&msg.subject),
            SysField::ContentType => opt_str(&msg.content_type),
            SysField::To => opt_str(&msg.to),
            SysField::ReplyTo => opt_str(&msg.reply_to),
            SysField::SequenceNumber => FieldValue::Num(msg.sequence_number as f64),
            SysField::EnqueuedTime => msg
                .enqueued_time
                .map(FieldValue::Time)
                .unwrap_or(FieldValue::Null),
            SysField::ScheduledEnqueueTime => msg
                .scheduled_enqueue_time
                .map(FieldValue::Time)
                .unwrap_or(FieldValue::Null),
            SysField::DeliveryCount => FieldValue::Num(f64::from(msg.delivery_count)),
            SysField::DeadLetterReason => opt_str(&msg.dead_letter_reason),
            SysField::DeadLetterErrorDescription => opt_str(&msg.dead_letter_description),
            SysField::DeadLetterSource => opt_str(&msg.dead_letter_source),
            SysField::State => FieldValue::Str(msg.state.as_str().to_string()),
            SysField::Size => FieldValue::Num(msg.body.len() as f64),
        },
        Field::User(name) => match msg.application_properties.get(name) {
            None | Some(PropValue::Null) => FieldValue::Null,
            Some(PropValue::Bool(b)) => FieldValue::Bool(*b),
            Some(PropValue::Int(n)) => FieldValue::Num(*n as f64),
            Some(PropValue::UInt(n)) => FieldValue::Num(*n as f64),
            Some(PropValue::Double(d)) => FieldValue::Num(*d),
            Some(PropValue::String(s)) | Some(PropValue::Uuid(s)) | Some(PropValue::Other(s)) => {
                FieldValue::Str(s.clone())
            }
            Some(PropValue::Timestamp(t)) => FieldValue::Time(*t),
            Some(PropValue::Binary(b)) => {
                FieldValue::Str(base64::engine::general_purpose::STANDARD.encode(b))
            }
        },
    }
}

fn order(fv: &FieldValue, lit: &Lit) -> Option<Ordering> {
    match (fv, lit) {
        (FieldValue::Num(a), Lit::Num(b)) => a.partial_cmp(b),
        (FieldValue::Str(a), Lit::Num(b)) => a.trim().parse::<f64>().ok()?.partial_cmp(b),
        (FieldValue::Str(a), Lit::Str { text, .. }) => Some(a.as_str().cmp(text.as_str())),
        (FieldValue::Time(a), Lit::Str { time, .. }) => Some(a.cmp(&(*time)?)),
        (FieldValue::Bool(a), Lit::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn compare(fv: &FieldValue, op: CmpOp, lit: &Lit) -> bool {
    let Some(ord) = order(fv, lit) else {
        return false;
    };
    match op {
        CmpOp::Eq => ord == Ordering::Equal,
        CmpOp::Ne => ord != Ordering::Equal,
        CmpOp::Lt => ord == Ordering::Less,
        CmpOp::Le => ord != Ordering::Greater,
        CmpOp::Gt => ord == Ordering::Greater,
        CmpOp::Ge => ord != Ordering::Less,
    }
}

/// Case-insensitive comparison of single characters (full Unicode lowercase mapping).
fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// `%` = any run, `_` = exactly one character; everything else compares
/// case-insensitively, one character at a time.
fn like(text: &str, pattern: &[char]) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p = pattern;
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '%' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && (p[pi] == '_' || same_char(p[pi], t[ti])) {
            ti += 1;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

fn eval(expr: &Expr, msg: &PeekedMessage) -> bool {
    match expr {
        Expr::And(a, b) => eval(a, msg) && eval(b, msg),
        Expr::Or(a, b) => eval(a, msg) || eval(b, msg),
        Expr::Not(e) => !eval(e, msg),
        Expr::Cmp(f, op, lit) => compare(&field_value(msg, f), *op, lit),
        Expr::IsNull(f, negated) => (field_value(msg, f) == FieldValue::Null) != *negated,
        Expr::Like(f, pattern, negated) => {
            let text = match field_value(msg, f) {
                FieldValue::Str(s) => s,
                FieldValue::Num(n) => n.to_string(),
                FieldValue::Time(t) => t.format(&Rfc3339).unwrap_or_default(),
                FieldValue::Bool(b) => b.to_string(),
                FieldValue::Null => return false,
            };
            like(&text, pattern) != *negated
        }
        Expr::In(f, values, negated) => {
            let fv = field_value(msg, f);
            if fv == FieldValue::Null {
                return false;
            }
            values.iter().any(|v| compare(&fv, CmpOp::Eq, v)) != *negated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::lexer::lex;
    use crate::messaging::test_message;
    use crate::messaging::{MessageBody, MessageState, PropValue};
    use time::OffsetDateTime;

    fn msg() -> PeekedMessage {
        let mut m = test_message(42);
        m.subject = Some("order.created".into());
        m.message_id = Some("m-1".into());
        m.delivery_count = 3;
        m.state = MessageState::Scheduled;
        m.enqueued_time = Some(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap());
        m.application_properties
            .insert("tenant".into(), PropValue::Int(7));
        m.application_properties
            .insert("city".into(), PropValue::String("Pisa".into()));
        m.application_properties
            .insert("amount".into(), PropValue::String("12.5".into()));
        m.body = MessageBody::Text("hello".into());
        m
    }

    fn eval(src: &str) -> bool {
        PropertyFilter::parse(src).unwrap().matches(&msg())
    }

    #[test]
    fn system_and_user_properties() {
        assert!(eval("sys.Subject = 'order.created'"));
        assert!(eval("sys.label = 'order.created'"));
        assert!(eval("sys.SequenceNumber = 42 AND tenant = 7"));
        assert!(eval("user.city = 'Pisa' AND NOT city = 'Rome'"));
        assert!(eval("sys.DeliveryCount >= 3 AND sys.DeliveryCount < 4"));
        assert!(eval("sys.State = 'scheduled'"));
        assert!(eval("sys.Size = 5"));
        assert!(!eval("tenant = 8"));
    }

    #[test]
    fn like_in_null_and_precedence() {
        assert!(eval("sys.Subject LIKE 'ORDER.%'"));
        assert!(eval("city LIKE 'P_sa'"));
        assert!(eval("city NOT LIKE 'R%'"));
        assert!(eval("city IN ('Rome', 'Pisa')"));
        assert!(eval("city NOT IN ('Rome')"));
        assert!(eval("missing IS NULL AND city IS NOT NULL"));
        assert!(eval("tenant = 1 OR tenant = 7 AND city = 'Pisa'"));
        assert!(!eval("(tenant = 1 OR tenant = 7) AND city = 'Rome'"));
    }

    #[test]
    fn numeric_strings_and_times() {
        assert!(eval("amount > 10"));
        assert!(eval("sys.EnqueuedTime > '2023-01-01T00:00:00Z'"));
        assert!(!eval("sys.EnqueuedTime > '2024-01-01T00:00:00Z'"));
    }

    #[test]
    fn type_mismatch_is_no_match() {
        assert!(!eval("city > 5"));
        assert!(!eval("tenant = 'seven'"));
        assert!(!eval("sys.EnqueuedTime = 'not a time'"));
        assert!(!eval("missing = 1"));
    }

    #[test]
    fn quotes_escape() {
        let mut m = msg();
        m.application_properties
            .insert("q".into(), PropValue::String("it's".into()));
        assert!(PropertyFilter::parse("q = 'it''s'").unwrap().matches(&m));
    }

    #[test]
    fn parse_errors_name_the_column() {
        for (src, needle) in [
            ("", "empty filter"),
            ("tenant =", "expected a value at column 9"),
            ("(tenant = 7", "expected `)` at column 12"),
            ("tenant = 7 tenant", "unexpected `tenant` at column 12"),
            ("sys.Nope = 1", "unknown system property `sys.Nope`"),
            ("tenant LIKE 5", "LIKE needs a 'string' pattern"),
        ] {
            let e = PropertyFilter::parse(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src:?} -> {e}");
        }
    }

    #[test]
    fn unicode_values_work() {
        let mut m = msg();
        m.application_properties
            .insert("name".into(), PropValue::String("Šejla".into()));
        assert!(
            PropertyFilter::parse("name LIKE 'še%'")
                .unwrap()
                .matches(&m)
        );
    }

    #[test]
    fn mid_input_errors_point_at_the_offending_token() {
        for (src, needle) in [
            ("(tenant = 7 city", "expected `)` at column 13"),
            (
                "tenant LIKE 5 AND x = 1",
                "LIKE needs a 'string' pattern at column 13",
            ),
            ("tenant IN 5", "IN needs a parenthesised list at column 11"),
            ("tenant IN (1 2)", "expected `,` or `)` at column 14"),
            ("tenant IN (1,", "expected a value at column 14"),
            ("tenant IS 5", "expected NULL at column 11"),
            (
                "tenant NOT = 5",
                "expected LIKE or IN after NOT at column 12",
            ),
            (
                "tenant 5",
                "expected a comparison operator, found `5` at column 8",
            ),
        ] {
            let e = PropertyFilter::parse(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src:?} -> {e}");
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let deep = format!("{}a = 1{}", "(".repeat(1_000), ")".repeat(1_000));
        assert!(
            PropertyFilter::parse(&deep)
                .unwrap_err()
                .to_string()
                .contains("too deeply nested")
        );
        let nots = format!("{}a = 1", "NOT ".repeat(1_000));
        assert!(
            PropertyFilter::parse(&nots)
                .unwrap_err()
                .to_string()
                .contains("too deeply nested")
        );
        let chain = vec!["a = 1"; 100_000].join(" OR ");
        assert!(
            PropertyFilter::parse(&chain)
                .unwrap_err()
                .to_string()
                .contains("too long")
        );
        for src in [
            "'",
            "-",
            "1e",
            "1.2.3",
            "a = 1e999",
            "x IN ()",
            "é",
            "a = '\u{0}",
            "AND",
            "NOT",
            "( )",
            "a IS NOT",
        ] {
            let _ = PropertyFilter::parse(src).map(|f| f.matches(&msg()));
        }
    }

    #[test]
    fn like_percent_is_a_wildcard_even_when_the_text_has_one() {
        let like_s = |t: &str, p: &str| like(t, &p.chars().collect::<Vec<_>>());
        assert!(like_s("%abc", "%c"));
        assert!(like_s("%x", "%"));
        assert!(like_s("100%", "100%"));
        assert!(like_s("", "%"));
        assert!(like_s("anything", "%"));
        assert!(like_s("aXbYbZc", "a%b%c"));
        assert!(!like_s("aXbYbZ", "a%b%c"));
        assert!(like_s("abc", "a_c"));
        assert!(!like_s("ac", "a_c"));
        assert!(like_s("a-b-c", "a_%c"));
        assert!(like_s("ab", "_%"));
        assert!(!like_s("", "_%"));
        assert!(like_s("a%b", "a%%b"));
    }

    #[test]
    fn like_folds_case_per_character() {
        let like_s = |t: &str, p: &str| like(t, &p.chars().collect::<Vec<_>>());
        // U+0130 lowercases to two chars; `_` must still match exactly one.
        assert!(like_s("\u{130}x", "_x"));
        assert!(!like_s("\u{130}x", "__x"));
        assert!(like_s("Šejla", "ŠEJLA"));
        assert!(like_s("STRASSE", "strasse"));
    }

    #[test]
    fn longest_accepted_chains_fit_a_small_stack() {
        // 1024 terms of 3 tokens plus 1023 operators = 4095 tokens, just under MAX_TOKENS.
        let terms = 1024;
        let or_chain = format!("{}tenant = 7", "tenant = 1 OR ".repeat(terms - 1));
        let and_chain = vec!["tenant = 7"; terms].join(" AND ");
        let and_false = format!(
            "{} AND tenant = 8",
            vec!["tenant = 7"; terms - 1].join(" AND ")
        );
        assert!(lex(&or_chain).unwrap().len() <= MAX_TOKENS);
        assert!(lex(&or_chain).unwrap().len() > MAX_TOKENS - 8);
        let result = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let m = msg();
                let a = PropertyFilter::parse(&or_chain).unwrap().matches(&m);
                let b = PropertyFilter::parse(&and_chain).unwrap().matches(&m);
                let c = PropertyFilter::parse(&and_false).unwrap().matches(&m);
                (a, b, c)
            })
            .unwrap()
            .join()
            .expect("no stack overflow");
        assert_eq!(result, (true, true, false));
    }

    #[test]
    fn rejects_non_finite_numbers_and_empty_prefixed_names() {
        for (src, needle) in [
            ("a = 1e999", "invalid number `1e999` at column 5"),
            ("a = -1e999", "invalid number `-1e999` at column 5"),
            (
                "user. = 1",
                "missing property name after `user.` at column 1",
            ),
            (
                "x = 1 AND sys. = 1",
                "missing property name after `sys.` at column 11",
            ),
        ] {
            let e = PropertyFilter::parse(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src:?} -> {e}");
        }
    }

    #[test]
    fn untrusted_text_is_escaped_in_errors() {
        let e = PropertyFilter::parse("a = 1 'x\ny'")
            .unwrap_err()
            .to_string();
        assert!(e.contains("'x\\ny'") && !e.contains('\n'), "{e}");
    }

    #[test]
    fn time_literals_are_parsed_once_and_bad_ones_never_match() {
        let f = PropertyFilter::parse("sys.EnqueuedTime >= '2023-11-14T22:13:20Z'").unwrap();
        assert!(f.matches(&msg()));
        assert!(
            !PropertyFilter::parse("sys.EnqueuedTime != 'garbage'")
                .unwrap()
                .matches(&msg())
        );
    }
}

//! Tokens for the property-filter language.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    /// 1-based column of the token's first character.
    pub col: usize,
}

pub(crate) fn err(col: usize, msg: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("filter: {msg} at column {col}"))
}

pub(crate) fn lex(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let tok = match c {
            '(' => {
                i += 1;
                Tok::LParen
            }
            ')' => {
                i += 1;
                Tok::RParen
            }
            ',' => {
                i += 1;
                Tok::Comma
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(err(col, "unterminated string")),
                        Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(*ch);
                            i += 1;
                        }
                    }
                }
                Tok::Str(s)
            }
            _ if two == "!=" || two == "<>" || two == "<=" || two == ">=" => {
                i += 2;
                Tok::Op(match two.as_str() {
                    "!=" | "<>" => "!=",
                    "<=" => "<=",
                    _ => ">=",
                })
            }
            '=' => {
                i += 1;
                Tok::Op("=")
            }
            '<' => {
                i += 1;
                Tok::Op("<")
            }
            '>' => {
                i += 1;
                Tok::Op(">")
            }
            _ if c.is_ascii_digit()
                || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                let start = i;
                i += 1;
                while chars
                    .get(i)
                    .is_some_and(|d| d.is_ascii_digit() || *d == '.' || *d == 'e' || *d == 'E')
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let n: f64 = text
                    .parse()
                    .ok()
                    .filter(|n: &f64| n.is_finite())
                    .ok_or_else(|| err(col, format!("invalid number `{text}`")))?;
                Tok::Num(n)
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|d| d.is_alphanumeric() || matches!(d, '_' | '.' | '-'))
                {
                    i += 1;
                }
                Tok::Ident(chars[start..i].iter().collect())
            }
            other => return Err(err(col, format!("unexpected character `{other}`"))),
        };
        out.push(Token { tok, col });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn lexes_operators_strings_numbers() {
        assert_eq!(
            toks("sys.DeliveryCount >= 3 AND name <> 'it''s'"),
            vec![
                Tok::Ident("sys.DeliveryCount".into()),
                Tok::Op(">="),
                Tok::Num(3.0),
                Tok::Ident("AND".into()),
                Tok::Ident("name".into()),
                Tok::Op("!="),
                Tok::Str("it's".into()),
            ]
        );
        assert_eq!(
            toks("x = -2.5"),
            vec![Tok::Ident("x".into()), Tok::Op("="), Tok::Num(-2.5)]
        );
    }

    #[test]
    fn unterminated_string_names_column() {
        let e = lex("a = 'oops").unwrap_err().to_string();
        assert!(e.contains("unterminated string at column 5"), "{e}");
    }

    #[test]
    fn unexpected_character_is_an_error() {
        assert!(lex("a = #").unwrap_err().to_string().contains("column 5"));
    }
}

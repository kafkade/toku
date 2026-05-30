use serde::{Deserialize, Serialize};
use std::fmt;

use crate::TokuError;

/// A saved smart-shelf filter that can be serialized to/from JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmartFilter {
    pub expression: FilterExpr,
}

/// A boolean expression tree of filter conditions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FilterExpr {
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
    Condition(FilterCondition),
}

/// A single filter predicate (e.g. `status:read`, `pages:>400`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilterCondition {
    pub field: FilterField,
    pub op: FilterOp,
    pub value: String,
}

/// Filterable fields for smart shelves.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterField {
    Status,
    Tag,
    Mood,
    Pace,
    Rating,
    Pages,
    Author,
    Format,
    Shelf,
    PubDate,
    DateAdded,
}

impl FilterField {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Tag => "tag",
            Self::Mood => "mood",
            Self::Pace => "pace",
            Self::Rating => "rating",
            Self::Pages => "pages",
            Self::Author => "author",
            Self::Format => "format",
            Self::Shelf => "shelf",
            Self::PubDate => "pub_date",
            Self::DateAdded => "date_added",
        }
    }

    fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "status" => Some(Self::Status),
            "tag" => Some(Self::Tag),
            "mood" => Some(Self::Mood),
            "pace" => Some(Self::Pace),
            "rating" => Some(Self::Rating),
            "pages" | "page_count" => Some(Self::Pages),
            "author" => Some(Self::Author),
            "format" => Some(Self::Format),
            "shelf" => Some(Self::Shelf),
            "pub_date" | "published" => Some(Self::PubDate),
            "date_added" | "added" | "created" => Some(Self::DateAdded),
            _ => None,
        }
    }

    fn is_numeric(&self) -> bool {
        matches!(self, Self::Rating | Self::Pages)
    }

    fn is_date(&self) -> bool {
        matches!(self, Self::PubDate | Self::DateAdded)
    }

    fn supports_comparison(&self) -> bool {
        self.is_numeric() || self.is_date()
    }
}

/// Comparison operator for filter conditions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl FilterOp {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

impl fmt::Display for FilterOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eq => Ok(()),
            Self::Gt => write!(f, ">"),
            Self::Gte => write!(f, ">="),
            Self::Lt => write!(f, "<"),
            Self::Lte => write!(f, "<="),
        }
    }
}

impl SmartFilter {
    /// Parse a filter DSL string into a SmartFilter.
    ///
    /// Grammar:
    /// ```text
    /// filter     = or_expr
    /// or_expr    = and_expr ("OR" and_expr)*
    /// and_expr   = atom ("AND" atom)*
    /// atom       = "(" or_expr ")" | condition
    /// condition  = field ":" op? value
    /// value      = quoted_string | unquoted_word
    /// ```
    pub fn parse(input: &str) -> Result<Self, TokuError> {
        let tokens = tokenize(input)?;
        let mut pos = 0;
        let expr = parse_or_expr(&tokens, &mut pos)?;
        if pos < tokens.len() {
            return Err(TokuError::InvalidFilter(format!(
                "unexpected token '{}' at position {pos}",
                tokens[pos]
            )));
        }
        Ok(SmartFilter { expression: expr })
    }

    /// Serialize to JSON for database storage.
    pub fn to_json(&self) -> Result<String, TokuError> {
        serde_json::to_string(self)
            .map_err(|e| TokuError::InvalidFilter(format!("failed to serialize filter: {e}")))
    }

    /// Deserialize from JSON stored in the database.
    pub fn from_json(json: &str) -> Result<Self, TokuError> {
        serde_json::from_str(json)
            .map_err(|e| TokuError::InvalidFilter(format!("invalid filter JSON: {e}")))
    }
}

impl fmt::Display for SmartFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.expression)
    }
}

impl fmt::Display for FilterExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterExpr::Condition(c) => write!(f, "{}", c),
            FilterExpr::And(exprs) => {
                for (i, expr) in exprs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " AND ")?;
                    }
                    let needs_parens = matches!(expr, FilterExpr::Or(_));
                    if needs_parens {
                        write!(f, "({})", expr)?;
                    } else {
                        write!(f, "{}", expr)?;
                    }
                }
                Ok(())
            }
            FilterExpr::Or(exprs) => {
                for (i, expr) in exprs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " OR ")?;
                    }
                    write!(f, "{}", expr)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for FilterCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}{}", self.field.as_str(), self.op, self.value)
    }
}

// --- Tokenizer ---

fn tokenize(input: &str) -> Result<Vec<String>, TokuError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip whitespace
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }

        // Parentheses
        if chars[i] == '(' || chars[i] == ')' {
            tokens.push(chars[i].to_string());
            i += 1;
            continue;
        }

        // Quoted string (part of a condition value, but we tokenize the whole condition)
        // Actually, we tokenize words and special chars separately
        // Collect a word (non-whitespace, non-paren sequence)
        let start = i;
        if chars[i] == '"' {
            // Quoted string
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            if i >= chars.len() {
                return Err(TokuError::InvalidFilter(
                    "unterminated quoted string".to_string(),
                ));
            }
            i += 1; // skip closing quote
            tokens.push(chars[start..i].iter().collect());
        } else {
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '(' && chars[i] != ')'
            {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
        }
    }

    Ok(tokens)
}

// --- Recursive descent parser ---

fn parse_or_expr(tokens: &[String], pos: &mut usize) -> Result<FilterExpr, TokuError> {
    let mut exprs = vec![parse_and_expr(tokens, pos)?];

    while *pos < tokens.len() && tokens[*pos].eq_ignore_ascii_case("OR") {
        *pos += 1;
        exprs.push(parse_and_expr(tokens, pos)?);
    }

    if exprs.len() == 1 {
        Ok(exprs.remove(0))
    } else {
        Ok(FilterExpr::Or(exprs))
    }
}

fn parse_and_expr(tokens: &[String], pos: &mut usize) -> Result<FilterExpr, TokuError> {
    let mut exprs = vec![parse_atom(tokens, pos)?];

    while *pos < tokens.len() && tokens[*pos].eq_ignore_ascii_case("AND") {
        *pos += 1;
        exprs.push(parse_atom(tokens, pos)?);
    }

    if exprs.len() == 1 {
        Ok(exprs.remove(0))
    } else {
        Ok(FilterExpr::And(exprs))
    }
}

fn parse_atom(tokens: &[String], pos: &mut usize) -> Result<FilterExpr, TokuError> {
    if *pos >= tokens.len() {
        return Err(TokuError::InvalidFilter(
            "unexpected end of filter expression".to_string(),
        ));
    }

    // Grouped expression: ( or_expr )
    if tokens[*pos] == "(" {
        *pos += 1;
        let expr = parse_or_expr(tokens, pos)?;
        if *pos >= tokens.len() || tokens[*pos] != ")" {
            return Err(TokuError::InvalidFilter(
                "missing closing parenthesis".to_string(),
            ));
        }
        *pos += 1;
        return Ok(expr);
    }

    // Condition: field:op?value
    parse_condition(tokens, pos)
}

fn parse_condition(tokens: &[String], pos: &mut usize) -> Result<FilterExpr, TokuError> {
    let token = &tokens[*pos];
    *pos += 1;

    // Find the colon separator
    let colon_idx = token.find(':').ok_or_else(|| {
        TokuError::InvalidFilter(format!("expected 'field:value' condition, got '{token}'"))
    })?;

    let field_str = &token[..colon_idx];
    let rest = &token[colon_idx + 1..];

    let field = FilterField::from_str_loose(field_str)
        .ok_or_else(|| TokuError::InvalidFilter(format!("unknown filter field '{field_str}'")))?;

    // Parse operator prefix from rest
    let (op, value_str) = if let Some(v) = rest.strip_prefix(">=") {
        (FilterOp::Gte, v)
    } else if let Some(v) = rest.strip_prefix("<=") {
        (FilterOp::Lte, v)
    } else if let Some(v) = rest.strip_prefix('>') {
        (FilterOp::Gt, v)
    } else if let Some(v) = rest.strip_prefix('<') {
        (FilterOp::Lt, v)
    } else if let Some(v) = rest.strip_prefix('=') {
        (FilterOp::Eq, v)
    } else {
        (FilterOp::Eq, rest)
    };

    // Value might be in the next token if this token ended at the colon or is a quoted string
    let value = if value_str.is_empty() {
        // Value is in the next token (e.g., filter was tokenized as "field:" "value")
        if *pos < tokens.len()
            && !is_keyword(&tokens[*pos])
            && tokens[*pos] != "("
            && tokens[*pos] != ")"
        {
            let v = tokens[*pos].clone();
            *pos += 1;
            strip_quotes(&v)
        } else {
            return Err(TokuError::InvalidFilter(format!(
                "missing value for field '{field_str}'"
            )));
        }
    } else {
        strip_quotes(value_str)
    };

    if value.is_empty() {
        return Err(TokuError::InvalidFilter(format!(
            "empty value for field '{field_str}'"
        )));
    }

    // Validate operator/value compatibility
    if op != FilterOp::Eq && !field.supports_comparison() {
        return Err(TokuError::InvalidFilter(format!(
            "comparison operators are only supported for numeric and date fields, not '{field_str}'"
        )));
    }

    if field.is_numeric() {
        value.parse::<i64>().map_err(|_| {
            TokuError::InvalidFilter(format!(
                "field '{field_str}' requires a numeric value, got '{value}'"
            ))
        })?;
    }

    // Validate date format (YYYY or YYYY-MM-DD)
    if field.is_date() {
        validate_date_value(&value, field_str)?;
    }

    Ok(FilterExpr::Condition(FilterCondition { field, op, value }))
}

fn is_keyword(token: &str) -> bool {
    token.eq_ignore_ascii_case("AND") || token.eq_ignore_ascii_case("OR")
}

fn strip_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn validate_date_value(value: &str, field_name: &str) -> Result<(), TokuError> {
    // Accept YYYY or YYYY-MM-DD
    if value.len() == 4 && value.parse::<u16>().is_ok() {
        return Ok(());
    }
    if value.len() == 10 {
        let parts: Vec<&str> = value.split('-').collect();
        if parts.len() == 3
            && parts[0].len() == 4
            && parts[1].len() == 2
            && parts[2].len() == 2
            && parts[0].parse::<u16>().is_ok()
            && parts[1].parse::<u8>().is_ok()
            && parts[2].parse::<u8>().is_ok()
        {
            return Ok(());
        }
    }
    Err(TokuError::InvalidFilter(format!(
        "field '{field_name}' requires a date value (YYYY or YYYY-MM-DD), got '{value}'"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_status() {
        let f = SmartFilter::parse("status:want_to_read").unwrap();
        assert_eq!(
            f.expression,
            FilterExpr::Condition(FilterCondition {
                field: FilterField::Status,
                op: FilterOp::Eq,
                value: "want_to_read".to_string(),
            })
        );
    }

    #[test]
    fn parse_rating_comparison() {
        let f = SmartFilter::parse("rating:>=9").unwrap();
        assert_eq!(
            f.expression,
            FilterExpr::Condition(FilterCondition {
                field: FilterField::Rating,
                op: FilterOp::Gte,
                value: "9".to_string(),
            })
        );
    }

    #[test]
    fn parse_pages_gt() {
        let f = SmartFilter::parse("pages:>400").unwrap();
        assert_eq!(
            f.expression,
            FilterExpr::Condition(FilterCondition {
                field: FilterField::Pages,
                op: FilterOp::Gt,
                value: "400".to_string(),
            })
        );
    }

    #[test]
    fn parse_and_expression() {
        let f = SmartFilter::parse("status:want_to_read AND tag:sci-fi").unwrap();
        match &f.expression {
            FilterExpr::And(exprs) => {
                assert_eq!(exprs.len(), 2);
                assert!(
                    matches!(&exprs[0], FilterExpr::Condition(c) if c.field == FilterField::Status)
                );
                assert!(
                    matches!(&exprs[1], FilterExpr::Condition(c) if c.field == FilterField::Tag)
                );
            }
            _ => panic!("expected AND expression"),
        }
    }

    #[test]
    fn parse_or_expression() {
        let f = SmartFilter::parse("tag:sci-fi OR tag:fantasy").unwrap();
        match &f.expression {
            FilterExpr::Or(exprs) => {
                assert_eq!(exprs.len(), 2);
            }
            _ => panic!("expected OR expression"),
        }
    }

    #[test]
    fn parse_and_or_precedence() {
        // AND binds tighter: status:read AND tag:sci-fi OR tag:fantasy
        // = (status:read AND tag:sci-fi) OR tag:fantasy
        let f = SmartFilter::parse("status:read AND tag:sci-fi OR tag:fantasy").unwrap();
        match &f.expression {
            FilterExpr::Or(exprs) => {
                assert_eq!(exprs.len(), 2);
                assert!(matches!(&exprs[0], FilterExpr::And(_)));
                assert!(matches!(&exprs[1], FilterExpr::Condition(_)));
            }
            _ => panic!("expected OR expression with AND child"),
        }
    }

    #[test]
    fn parse_parenthesized_or() {
        let f = SmartFilter::parse("status:want_to_read AND (tag:sci-fi OR tag:fantasy)").unwrap();
        match &f.expression {
            FilterExpr::And(exprs) => {
                assert_eq!(exprs.len(), 2);
                assert!(matches!(&exprs[0], FilterExpr::Condition(_)));
                assert!(matches!(&exprs[1], FilterExpr::Or(_)));
            }
            _ => panic!("expected AND expression with OR child"),
        }
    }

    #[test]
    fn parse_complex_filter() {
        let f = SmartFilter::parse(
            "status:want_to_read AND pages:>300 AND (tag:sci-fi OR tag:fantasy)",
        )
        .unwrap();
        match &f.expression {
            FilterExpr::And(exprs) => assert_eq!(exprs.len(), 3),
            _ => panic!("expected AND with 3 children"),
        }
    }

    #[test]
    fn parse_mood_and_pace() {
        let f = SmartFilter::parse("mood:dark AND pace:fast").unwrap();
        match &f.expression {
            FilterExpr::And(exprs) => {
                assert_eq!(exprs.len(), 2);
                assert!(
                    matches!(&exprs[0], FilterExpr::Condition(c) if c.field == FilterField::Mood)
                );
                assert!(
                    matches!(&exprs[1], FilterExpr::Condition(c) if c.field == FilterField::Pace)
                );
            }
            _ => panic!("expected AND"),
        }
    }

    #[test]
    fn parse_author_filter() {
        let f = SmartFilter::parse("author:tolkien").unwrap();
        assert!(matches!(
            &f.expression,
            FilterExpr::Condition(c) if c.field == FilterField::Author && c.value == "tolkien"
        ));
    }

    #[test]
    fn parse_date_filter() {
        let f = SmartFilter::parse("pub_date:>=2020").unwrap();
        assert!(matches!(
            &f.expression,
            FilterExpr::Condition(c)
                if c.field == FilterField::PubDate
                && c.op == FilterOp::Gte
                && c.value == "2020"
        ));
    }

    #[test]
    fn parse_date_added_full() {
        let f = SmartFilter::parse("date_added:>=2024-01-01").unwrap();
        assert!(matches!(
            &f.expression,
            FilterExpr::Condition(c) if c.field == FilterField::DateAdded && c.value == "2024-01-01"
        ));
    }

    #[test]
    fn parse_field_aliases() {
        SmartFilter::parse("published:>=2020").unwrap();
        SmartFilter::parse("added:>=2024-01-01").unwrap();
        SmartFilter::parse("created:>=2024-01-01").unwrap();
        SmartFilter::parse("page_count:>100").unwrap();
    }

    #[test]
    fn error_unknown_field() {
        let err = SmartFilter::parse("foobar:value").unwrap_err();
        assert!(err.to_string().contains("unknown filter field"));
    }

    #[test]
    fn error_comparison_on_text_field() {
        let err = SmartFilter::parse("tag:>sci-fi").unwrap_err();
        assert!(err.to_string().contains("comparison operators"));
    }

    #[test]
    fn error_non_numeric_rating() {
        let err = SmartFilter::parse("rating:abc").unwrap_err();
        assert!(err.to_string().contains("numeric value"));
    }

    #[test]
    fn error_missing_value() {
        let err = SmartFilter::parse("status:").unwrap_err();
        assert!(err.to_string().contains("missing value"));
    }

    #[test]
    fn error_missing_colon() {
        let err = SmartFilter::parse("status").unwrap_err();
        assert!(err.to_string().contains("field:value"));
    }

    #[test]
    fn error_unclosed_paren() {
        let err = SmartFilter::parse("(status:read AND tag:sci-fi").unwrap_err();
        assert!(err.to_string().contains("parenthesis"));
    }

    #[test]
    fn error_invalid_date() {
        let err = SmartFilter::parse("pub_date:not-a-date").unwrap_err();
        assert!(err.to_string().contains("date value"));
    }

    #[test]
    fn json_round_trip() {
        let f = SmartFilter::parse("status:read AND rating:>=8").unwrap();
        let json = f.to_json().unwrap();
        let f2 = SmartFilter::from_json(&json).unwrap();
        assert_eq!(f, f2);
    }

    #[test]
    fn display_round_trip() {
        let input = "status:read AND (tag:sci-fi OR tag:fantasy)";
        let f = SmartFilter::parse(input).unwrap();
        let display = f.to_string();
        // Re-parse the display output
        let f2 = SmartFilter::parse(&display).unwrap();
        assert_eq!(f, f2);
    }
}

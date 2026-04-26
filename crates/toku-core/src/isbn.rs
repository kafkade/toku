use crate::TokuError;

/// A validated ISBN (either ISBN-10 or ISBN-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isbn {
    Isbn10(String),
    Isbn13(String),
}

impl Isbn {
    /// Parse and validate an ISBN string (10 or 13 digits).
    pub fn parse(input: &str) -> Result<Self, TokuError> {
        let cleaned: String = input
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X')
            .collect();

        match cleaned.len() {
            10 => {
                if validate_isbn10(&cleaned) {
                    Ok(Isbn::Isbn10(cleaned))
                } else {
                    Err(TokuError::InvalidIsbn(format!(
                        "invalid ISBN-10 check digit: {input}"
                    )))
                }
            }
            13 => {
                if validate_isbn13(&cleaned) {
                    Ok(Isbn::Isbn13(cleaned))
                } else {
                    Err(TokuError::InvalidIsbn(format!(
                        "invalid ISBN-13 check digit: {input}"
                    )))
                }
            }
            _ => Err(TokuError::InvalidIsbn(format!(
                "ISBN must be 10 or 13 digits, got {}: {input}",
                cleaned.len()
            ))),
        }
    }

    /// Convert this ISBN to ISBN-13. ISBN-13 returns itself.
    /// Only 978-prefixed ISBN-10s can be converted.
    pub fn to_isbn13(&self) -> String {
        match self {
            Isbn::Isbn13(s) => s.clone(),
            Isbn::Isbn10(s) => {
                let without_check = &s[..9];
                let base = format!("978{without_check}");
                let check = compute_isbn13_check(&base);
                format!("{base}{check}")
            }
        }
    }

    /// Convert this ISBN to ISBN-10 if possible. Only 978-prefixed ISBN-13s convert.
    pub fn to_isbn10(&self) -> Option<String> {
        match self {
            Isbn::Isbn10(s) => Some(s.clone()),
            Isbn::Isbn13(s) => {
                if !s.starts_with("978") {
                    return None;
                }
                let core = &s[3..12];
                let check = compute_isbn10_check(core);
                Some(format!("{core}{check}"))
            }
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Isbn::Isbn10(s) | Isbn::Isbn13(s) => s,
        }
    }
}

impl std::fmt::Display for Isbn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn validate_isbn10(digits: &str) -> bool {
    if digits.len() != 10 {
        return false;
    }
    let sum: u32 = digits
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let val = if c == 'X' {
                10
            } else {
                c.to_digit(10).unwrap_or(0)
            };
            val * (10 - i as u32)
        })
        .sum();
    sum.is_multiple_of(11)
}

fn validate_isbn13(digits: &str) -> bool {
    if digits.len() != 13 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let sum: u32 = digits
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let val = c.to_digit(10).unwrap_or(0);
            if i % 2 == 0 { val } else { val * 3 }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn compute_isbn13_check(first_12: &str) -> char {
    let sum: u32 = first_12
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let val = c.to_digit(10).unwrap_or(0);
            if i % 2 == 0 { val } else { val * 3 }
        })
        .sum();
    let check = (10 - (sum % 10)) % 10;
    char::from_digit(check, 10).unwrap()
}

fn compute_isbn10_check(first_9: &str) -> char {
    let sum: u32 = first_9
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let val = c.to_digit(10).unwrap_or(0);
            val * (10 - i as u32)
        })
        .sum();
    let check = (11 - (sum % 11)) % 11;
    if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_isbn13() {
        let isbn = Isbn::parse("9780441013593").unwrap();
        assert!(matches!(isbn, Isbn::Isbn13(_)));
        assert_eq!(isbn.as_str(), "9780441013593");
    }

    #[test]
    fn valid_isbn10() {
        let isbn = Isbn::parse("0441013597").unwrap();
        assert!(matches!(isbn, Isbn::Isbn10(_)));
    }

    #[test]
    fn isbn10_with_x_check() {
        let isbn = Isbn::parse("080442957X").unwrap();
        assert!(matches!(isbn, Isbn::Isbn10(_)));
    }

    #[test]
    fn isbn_strips_hyphens() {
        let isbn = Isbn::parse("978-0-441-01359-3").unwrap();
        assert_eq!(isbn.as_str(), "9780441013593");
    }

    #[test]
    fn isbn10_to_isbn13() {
        let isbn = Isbn::parse("0441013597").unwrap();
        assert_eq!(isbn.to_isbn13(), "9780441013593");
    }

    #[test]
    fn isbn13_to_isbn10() {
        let isbn = Isbn::parse("9780441013593").unwrap();
        assert_eq!(isbn.to_isbn10(), Some("0441013597".to_string()));
    }

    #[test]
    fn isbn13_979_no_isbn10() {
        // 979-prefix ISBN-13s cannot be converted to ISBN-10
        let isbn = Isbn::parse("9791032305690").unwrap();
        assert!(isbn.to_isbn10().is_none());
    }

    #[test]
    fn invalid_isbn_check_digit() {
        assert!(Isbn::parse("9780441013590").is_err());
    }

    #[test]
    fn invalid_isbn_length() {
        assert!(Isbn::parse("12345").is_err());
    }
}

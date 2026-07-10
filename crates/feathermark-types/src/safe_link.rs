use std::fmt;

use thiserror::Error;
use url::Url;

pub const MAX_SAFE_LINK_BYTES: usize = 768;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SafeLinkTarget {
    canonical: String,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SafeLinkError {
    #[error("link target is not UTF-8")]
    InvalidUtf8,
    #[error("link target contains a forbidden character")]
    ForbiddenCharacter,
    #[error("link target contains an invalid percent escape")]
    InvalidPercentEscape,
    #[error("link target contains a forbidden or noncanonical percent escape")]
    ForbiddenPercentEscape,
    #[error("link target is not an allowed absolute URL")]
    InvalidUrl,
    #[error("URL scheme is not allowed")]
    ForbiddenScheme,
    #[error("URL userinfo is forbidden")]
    UserInfo,
    #[error("canonical URL exceeds {MAX_SAFE_LINK_BYTES} bytes")]
    TooLong,
    #[error("wire URL is not canonical")]
    NonCanonicalWire,
}

impl SafeLinkTarget {
    pub fn parse(input: &str) -> Result<Self, SafeLinkError> {
        let decoded = html_escape::decode_html_entities(input);
        let decoded = decoded.as_ref();
        validate_characters(decoded)?;
        validate_percent_escapes(decoded)?;

        let parsed = Url::parse(decoded).map_err(|_| SafeLinkError::InvalidUrl)?;
        match parsed.scheme() {
            "http" | "https" => {
                if parsed.host_str().is_none() {
                    return Err(SafeLinkError::InvalidUrl);
                }
            }
            "mailto" => {
                if parsed.path().is_empty() {
                    return Err(SafeLinkError::InvalidUrl);
                }
            }
            _ => return Err(SafeLinkError::ForbiddenScheme),
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(SafeLinkError::UserInfo);
        }

        let canonical: String = parsed.into();
        if canonical.len() > MAX_SAFE_LINK_BYTES {
            return Err(SafeLinkError::TooLong);
        }
        validate_characters(&canonical)?;
        validate_percent_escapes(&canonical)?;
        Ok(Self { canonical })
    }

    pub fn parse_wire(input: &str) -> Result<Self, SafeLinkError> {
        let parsed = Self::parse(input)?;
        if parsed.as_canonical_str().as_bytes() != input.as_bytes() {
            return Err(SafeLinkError::NonCanonicalWire);
        }
        Ok(parsed)
    }

    pub fn parse_bytes(input: &[u8]) -> Result<Self, SafeLinkError> {
        Self::parse(std::str::from_utf8(input).map_err(|_| SafeLinkError::InvalidUtf8)?)
    }

    pub fn as_canonical_str(&self) -> &str {
        &self.canonical
    }
}

impl fmt::Display for SafeLinkTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_canonical_str())
    }
}

fn validate_characters(value: &str) -> Result<(), SafeLinkError> {
    if value.is_empty()
        || value.bytes().any(|byte| {
            !byte.is_ascii()
                || byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || byte == b'\\'
        })
    {
        return Err(SafeLinkError::ForbiddenCharacter);
    }
    Ok(())
}

fn validate_percent_escapes(value: &str) -> Result<(), SafeLinkError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(SafeLinkError::InvalidPercentEscape);
            }
            let decoded = (hex_value(bytes[index + 1]) << 4) | hex_value(bytes[index + 2]);
            if !decoded.is_ascii()
                || decoded.is_ascii_control()
                || decoded.is_ascii_whitespace()
                || decoded == b'\\'
                || decoded == b'%'
                || decoded.is_ascii_alphanumeric()
                || matches!(decoded, b'-' | b'.' | b'_' | b'~')
            {
                return Err(SafeLinkError::ForbiddenPercentEscape);
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("validated hexadecimal digit"),
    }
}

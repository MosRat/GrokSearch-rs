use grok_search_types::{GrokSearchError, Result};

use crate::{truncate_content, ParsedContent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentParseOptions {
    pub max_chars: Option<usize>,
}

pub trait ByteContentParser {
    fn kind(&self) -> ContentKind;

    fn parse_bytes(&self, bytes: &[u8], options: ContentParseOptions) -> Result<ParsedContent>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlainTextParser;

impl ByteContentParser for PlainTextParser {
    fn kind(&self) -> ContentKind {
        ContentKind::PlainText
    }

    fn parse_bytes(&self, bytes: &[u8], options: ContentParseOptions) -> Result<ParsedContent> {
        let content = decode_utf8(bytes, "plain text")?;
        Ok(truncate_content(content, options.max_chars))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownParser;

impl ByteContentParser for MarkdownParser {
    fn kind(&self) -> ContentKind {
        ContentKind::Markdown
    }

    fn parse_bytes(&self, bytes: &[u8], options: ContentParseOptions) -> Result<ParsedContent> {
        let content = decode_utf8(bytes, "markdown")?;
        Ok(truncate_content(content, options.max_chars))
    }
}

fn decode_utf8(bytes: &[u8], label: &str) -> Result<String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|err| GrokSearchError::Parse(format!("{label} is not valid UTF-8: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_parser_truncates_utf8_content() {
        let parsed = PlainTextParser
            .parse_bytes(b"abcdef", ContentParseOptions { max_chars: Some(4) })
            .expect("parse text");
        assert_eq!(parsed.content, "abcd");
        assert!(parsed.truncated);
    }

    #[test]
    fn markdown_parser_preserves_markdown_text() {
        let parsed = MarkdownParser
            .parse_bytes(b"# Paper\n\nBody", ContentParseOptions::default())
            .expect("parse markdown");
        assert_eq!(parsed.content, "# Paper\n\nBody");
        assert!(!parsed.truncated);
    }

    #[test]
    fn parser_rejects_invalid_utf8() {
        let err = PlainTextParser
            .parse_bytes(&[0xff], ContentParseOptions::default())
            .expect_err("invalid utf-8 should fail");
        assert!(matches!(err, GrokSearchError::Parse(_)));
    }
}

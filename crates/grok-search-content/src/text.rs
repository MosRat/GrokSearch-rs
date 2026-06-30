#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedContent {
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
}

pub fn truncate_content(content: String, max_chars: Option<usize>) -> ParsedContent {
    let original_length = content.chars().count();
    let mut truncated = false;
    let content = if let Some(limit) = max_chars {
        if original_length > limit {
            truncated = true;
            content.chars().take(limit).collect()
        } else {
            content
        }
    } else {
        content
    };
    ParsedContent {
        content,
        original_length,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_content_by_chars() {
        let parsed = truncate_content("abcdef".to_string(), Some(3));
        assert_eq!(parsed.content, "abc");
        assert_eq!(parsed.original_length, 6);
        assert!(parsed.truncated);
    }

    #[test]
    fn keeps_content_when_under_limit() {
        let parsed = truncate_content("abc".to_string(), Some(5));
        assert_eq!(parsed.content, "abc");
        assert_eq!(parsed.original_length, 3);
        assert!(!parsed.truncated);
    }
}

use grok_search_types::{GrokSearchError, Result};

pub fn validate_pdf_bytes(bytes: &[u8], max_bytes: usize) -> Result<()> {
    if bytes.len() > max_bytes {
        return Err(GrokSearchError::Provider(format!(
            "academic pdf exceeds max size: {} > {}",
            bytes.len(),
            max_bytes
        )));
    }
    if !bytes.starts_with(b"%PDF") {
        return Err(GrokSearchError::Provider(
            "resolved academic full text is not a PDF".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pdf_bytes() {
        assert!(validate_pdf_bytes(b"not-pdf", 100).is_err());
    }

    #[test]
    fn rejects_oversized_pdf() {
        assert!(validate_pdf_bytes(b"%PDF-1.7", 3).is_err());
    }
}


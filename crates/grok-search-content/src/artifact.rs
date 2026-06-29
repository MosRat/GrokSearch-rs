use std::path::Path;

use grok_search_types::{GrokSearchError, Result};

pub fn ensure_output_dir(path: &Path, kind: &str) -> Result<()> {
    if path.exists() && !path.is_dir() {
        return Err(GrokSearchError::InvalidParams(format!(
            "{kind} output path is not a directory: {}",
            path.display()
        )));
    }
    std::fs::create_dir_all(path)
        .map_err(|err| GrokSearchError::Io(format!("create {kind} directory: {err}")))
}

pub fn reject_existing_path(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        return Err(GrokSearchError::InvalidParams(format!(
            "{label} already exists: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn write_text_file_no_overwrite(path: impl AsRef<Path>, content: &str) -> Result<u64> {
    let path = path.as_ref().to_path_buf();
    reject_existing_path(&path, "artifact path")?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            GrokSearchError::Io(format!(
                "create artifact directory {}: {err}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(&path, content)
        .map_err(|err| GrokSearchError::Io(format!("write artifact {}: {err}", path.display())))?;
    std::fs::metadata(&path)
        .map(|metadata| metadata.len())
        .map_err(|err| GrokSearchError::Io(format!("stat artifact {}: {err}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_output_dir_and_rejects_file_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("nested");
        ensure_output_dir(&output, "test").expect("create output dir");
        assert!(output.is_dir());

        let file = dir.path().join("file");
        std::fs::write(&file, "").expect("seed file");
        let err = ensure_output_dir(&file, "test").expect_err("file path should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn writes_text_file_and_rejects_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("artifact.md");
        let bytes = write_text_file_no_overwrite(&path, "# Paper").expect("write artifact");
        assert_eq!(bytes, 7);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Paper");

        let err = write_text_file_no_overwrite(&path, "# Again")
            .expect_err("existing artifact should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }
}

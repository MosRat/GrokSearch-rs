use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    GithubIssue,
    GithubPull,
    Stackexchange,
    Arxiv,
    Wikipedia,
    Generic,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_serializes_to_required_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&SourceType::GithubIssue).unwrap(),
            "\"github_issue\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::GithubPull).unwrap(),
            "\"github_pull\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Stackexchange).unwrap(),
            "\"stackexchange\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Arxiv).unwrap(),
            "\"arxiv\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Wikipedia).unwrap(),
            "\"wikipedia\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Generic).unwrap(),
            "\"generic\""
        );
    }
}

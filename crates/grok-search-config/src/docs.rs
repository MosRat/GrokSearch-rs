use crate::schema::CONFIG_ITEMS;

pub const CONFIG_SCHEMA_START: &str = "<!-- config-schema:start -->";
pub const CONFIG_SCHEMA_END: &str = "<!-- config-schema:end -->";

pub fn generated_config_reference_markdown() -> String {
    let mut out = String::new();
    out.push_str(CONFIG_SCHEMA_START);
    out.push_str("\n");
    out.push_str("### Generated config reference\n\n");
    out.push_str("This section is generated from the Rust config schema in `grok-search-config`. Update the schema first, then refresh this block.\n\n");

    let mut current_group = "";
    for item in CONFIG_ITEMS {
        if item.group != current_group {
            current_group = item.group;
            out.push_str("#### ");
            out.push_str(current_group);
            out.push_str("\n\n");
            out.push_str("| TOML key | Env aliases | Default | Description |\n");
            out.push_str("|---|---|---|---|\n");
        }
        out.push_str("| `");
        out.push_str(item.key.toml_key);
        out.push_str("` | ");
        out.push_str(&format_aliases(item.key.env_aliases));
        out.push_str(" | ");
        out.push_str(&escape_cell(item.default_display));
        out.push_str(" | ");
        out.push_str(&escape_cell(first_doc_sentence(item.doc)));
        out.push_str(" |\n");
    }

    out.push('\n');
    out.push_str(CONFIG_SCHEMA_END);
    out
}

pub fn replace_generated_config_reference(document: &str) -> Option<String> {
    let start = document.find(CONFIG_SCHEMA_START)?;
    let end = document.find(CONFIG_SCHEMA_END)? + CONFIG_SCHEMA_END.len();
    let mut out = String::new();
    out.push_str(&document[..start]);
    out.push_str(&generated_config_reference_markdown());
    out.push_str(&document[end..]);
    Some(out)
}

fn format_aliases(aliases: &[&str]) -> String {
    aliases
        .iter()
        .map(|alias| format!("`{alias}`"))
        .collect::<Vec<_>>()
        .join("<br>")
}

fn first_doc_sentence(doc: &str) -> &str {
    doc.trim().lines().next().unwrap_or("").trim()
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_doc_generated_block_matches_schema() {
        let docs = include_str!("../../../docs/CONFIGURATION.md");
        let expected = replace_generated_config_reference(docs)
            .expect("docs must contain config schema markers");
        assert_eq!(
            docs, expected,
            "docs/CONFIGURATION.md config-schema block is stale"
        );
    }
}

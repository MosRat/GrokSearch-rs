use crate::schema::{ConfigItem, CONFIG_ITEMS};

pub(crate) fn build_config_template() -> String {
    let mut out = String::new();
    out.push_str("# grok-search-rs global configuration\n");
    out.push_str("# Default path:\n");
    out.push_str("#   Unix / macOS / Git Bash:   $HOME/.config/grok-search-rs/config.toml\n");
    out.push_str(
        "#   Windows (PowerShell/cmd):  %USERPROFILE%\\.config\\grok-search-rs\\config.toml\n",
    );
    out.push_str("# Override anywhere with $GROK_SEARCH_CONFIG=/abs/path/to/config.toml\n");
    out.push_str("#\n");
    out.push_str("# Precedence: process env > this file > built-in defaults.\n");
    out.push_str("# All keys below are commented out; uncomment and fill what you need.\n");
    out.push_str("# Unknown keys are rejected; typos surface as errors, not silent drops.\n\n");

    let mut current_group = "";
    for item in CONFIG_ITEMS {
        if item.group != current_group {
            if !current_group.is_empty() {
                out.push('\n');
            }
            current_group = item.group;
            out.push_str("# -- ");
            out.push_str(current_group);
            out.push_str(" --\n");
        }
        push_template_item(&mut out, item);
    }
    out
}

fn push_template_item(out: &mut String, item: &ConfigItem) {
    for line in item.doc.trim().lines() {
        out.push_str("# ");
        out.push_str(line.trim());
        out.push('\n');
    }
    out.push_str("# ");
    out.push_str(item.key.toml_key);
    out.push_str(" = ");
    out.push_str(item.sample_value);
    if item.default_display != "unset" {
        out.push_str(" # default: ");
        out.push_str(item.default_display);
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use crate::loader::{config_template, CONFIG_TEMPLATE};
    use crate::schema::CONFIG_ITEMS;

    #[test]
    fn config_template_snapshot_matches_schema_generator() {
        assert_eq!(CONFIG_TEMPLATE, config_template());
        for item in CONFIG_ITEMS {
            let key = format!("# {} = ", item.key.toml_key);
            assert!(
                CONFIG_TEMPLATE.contains(&key),
                "template missing commented key {}",
                item.key.toml_key
            );
        }
    }
}

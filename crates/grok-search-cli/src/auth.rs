use grok_search_config::{self as config, Config};

pub(crate) async fn run_login(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let store = grok_search_auth::oauth::login::login(&path, true).await?;
    println!("Login successful.");
    println!("Auth file: {}", path.display());
    if let Some(exp) = grok_search_auth::oauth::token_store::jwt_exp(&store.access_token) {
        println!("Access token expires at unix time: {exp}");
    }
    Ok(())
}

pub(crate) fn run_status(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let status = grok_search_auth::oauth::token_store::auth_status(&path);
    println!("grok-search-rs OAuth status");
    println!("  Auth file: {}", status.path.display());
    println!(
        "  Authenticated: {}",
        if status.authenticated { "yes" } else { "no" }
    );
    println!(
        "  Refresh token: {}",
        if status.refresh_token_present {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "  Access expires at: {}",
        status
            .access_expires_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "  Base URL: {}",
        status.base_url.unwrap_or_else(|| "unknown".to_string())
    );
    Ok(())
}

pub(crate) fn run_logout(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let removed = grok_search_auth::oauth::token_store::delete_token_store(&path)?;
    if removed {
        println!("Removed OAuth token file: {}", path.display());
    } else {
        println!("No OAuth token file found: {}", path.display());
    }
    Ok(())
}

fn resolve_auth_path(cfg: &Config) -> anyhow::Result<std::path::PathBuf> {
    cfg.grok_auth_file
        .clone()
        .or_else(config::auth_path)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve OAuth auth path; set GROK_SEARCH_AUTH_FILE"))
}

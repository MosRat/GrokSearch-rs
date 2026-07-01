use grok_search_config::Config;

pub(crate) async fn build_service() -> anyhow::Result<grok_search_service::SearchService> {
    let cfg = Config::try_load()?;
    build_service_from_config(cfg).await
}

pub(crate) async fn build_service_from_config(
    cfg: Config,
) -> anyhow::Result<grok_search_service::SearchService> {
    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await?;
    Ok(grok_search_runtime::new_with_http(
        cfg,
        http,
        proxy_diagnostics,
    )?)
}

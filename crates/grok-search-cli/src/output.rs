use serde_json::Value;

use crate::service::build_service;

pub(crate) async fn invoke_and_print<T: serde::Serialize>(
    name: &str,
    params: T,
    compact: bool,
) -> anyhow::Result<()> {
    let service = build_service().await?;
    let args = serde_json::to_value(params)?;
    let value = grok_search_tools::invoke_tool(&service, name, args).await?;
    print_json(value, compact)
}

pub(crate) fn print_json(value: Value, compact: bool) -> anyhow::Result<()> {
    let text = if compact {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    println!("{text}");
    Ok(())
}

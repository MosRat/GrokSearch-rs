use std::collections::HashSet;

use grok_search_net::http::{get_text_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_provider_core::WechatProvider;
use grok_search_types::{
    GrokSearchError, Result, WechatArticle, WechatArticleQuality, WechatSearchInput,
    WechatSearchOutput,
};
use reqwest::header::{
    HeaderName, ACCEPT, ACCEPT_LANGUAGE, COOKIE, REFERER, SET_COOKIE, USER_AGENT,
};
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

const SOGOU_BASE: &str = "https://weixin.sogou.com";
const SOGOU_WEIXIN: &str = "https://weixin.sogou.com/weixin";
const SOGOU_COOKIE_SEED: &str = "https://v.sogou.com/v?ie=utf8&query=&p=40030600";
const PROVIDER_LABEL: &str = "Sogou Weixin";
const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/126 Safari/537.36";

#[derive(Clone)]
pub struct WechatSearchProvider {
    client: Client,
    max_response_bytes: usize,
}

impl WechatSearchProvider {
    pub fn with_client(client: Client) -> Self {
        Self::with_client_and_limit(client, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub fn with_client_and_limit(client: Client, max_response_bytes: usize) -> Self {
        Self {
            client,
            max_response_bytes,
        }
    }

    async fn warm_cookies(&self) -> Option<String> {
        let mut request = self.client.get(SOGOU_COOKIE_SEED);
        for (name, value) in browser_headers(None, None) {
            request = request.header(name, value);
        }
        let response = request.send().await.ok()?;
        let cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.split(';').next())
            .filter(|value| !value.trim().is_empty())
            .map(str::trim)
            .collect::<Vec<_>>();
        (!cookies.is_empty()).then(|| cookies.join("; "))
    }

    async fn fetch_text(
        &self,
        url: &str,
        referer: Option<&str>,
        cookies: Option<&str>,
    ) -> Result<String> {
        let headers = browser_headers(referer, cookies);
        let header_refs = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.as_str()))
            .collect::<Vec<_>>();
        get_text_limited(
            &self.client,
            url,
            &header_refs,
            PROVIDER_LABEL,
            self.max_response_bytes,
        )
        .await
    }

    async fn search_page(&self, query: &str, page: usize, cookies: Option<&str>) -> Result<String> {
        let mut url = Url::parse(SOGOU_WEIXIN).expect("static URL");
        url.query_pairs_mut()
            .append_pair("type", "2")
            .append_pair("query", query)
            .append_pair("ie", "utf8")
            .append_pair("page", &page.to_string());
        self.fetch_text(url.as_str(), Some(SOGOU_BASE), cookies)
            .await
    }

    async fn resolve_article_url(
        &self,
        sogou_url: &str,
        cookies: Option<&str>,
    ) -> Result<Option<String>> {
        if sogou_url.trim().is_empty() {
            return Ok(None);
        }
        let html = self
            .fetch_text(sogou_url, Some(SOGOU_BASE), cookies)
            .await?;
        Ok(resolve_wechat_url_from_redirect_html(&html))
    }

    async fn fetch_article_content(
        &self,
        url: &str,
        max_chars: Option<usize>,
    ) -> Result<(String, usize, bool)> {
        let html = self.fetch_text(url, Some(SOGOU_BASE), None).await?;
        let content = extract_wechat_article_content(&html).ok_or_else(|| {
            GrokSearchError::Parse("wechat article content not found".to_string())
        })?;
        Ok(truncate_chars(content, max_chars))
    }
}

#[async_trait::async_trait]
impl WechatProvider for WechatSearchProvider {
    async fn search(&self, input: WechatSearchInput) -> Result<WechatSearchOutput> {
        let query = input.query.trim().to_string();
        if query.is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "wechat_search.query is required".to_string(),
            ));
        }

        let max_results = input.max_results.unwrap_or(10).clamp(1, 50);
        let pages = input.pages.unwrap_or(1).clamp(1, 10);
        let include_content = input.include_content.unwrap_or(true);
        let account = input
            .account
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let cookies = self.warm_cookies().await;

        let mut articles = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();

        for page in 1..=pages {
            let page_html = match self.search_page(&query, page, cookies.as_deref()).await {
                Ok(html) => html,
                Err(err) => {
                    warnings.push(format!("page {page}: {err}"));
                    continue;
                }
            };
            let mut page_items = parse_sogou_search_results(&page_html);
            if page_items.is_empty() {
                warnings.push(format!("page {page}: no article results parsed"));
            }

            for mut article in page_items.drain(..) {
                let source_match = account
                    .as_deref()
                    .map(|expected| article.source.trim() == expected)
                    .unwrap_or(true);
                article.quality.source_match = source_match;
                if !source_match {
                    continue;
                }

                match self
                    .resolve_article_url(&article.sogou_url, cookies.as_deref())
                    .await
                {
                    Ok(Some(url)) => {
                        article.quality.url_resolved = true;
                        article.url = Some(url);
                    }
                    Ok(None) => {
                        article
                            .quality
                            .warnings
                            .push("failed to resolve mp.weixin.qq.com URL".to_string());
                    }
                    Err(err) => {
                        article
                            .quality
                            .warnings
                            .push(format!("URL resolution failed: {err}"));
                    }
                }

                if include_content {
                    if let Some(url) = article.url.as_deref() {
                        match self
                            .fetch_article_content(url, input.max_content_chars)
                            .await
                        {
                            Ok((content, original_length, truncated)) => {
                                article.quality.content_fetched = true;
                                article.content = Some(content);
                                article.content_original_length = Some(original_length);
                                article.content_truncated = truncated;
                            }
                            Err(err) => article
                                .quality
                                .warnings
                                .push(format!("content fetch failed: {err}")),
                        }
                    } else {
                        article
                            .quality
                            .warnings
                            .push("content skipped because URL was not resolved".to_string());
                    }
                }

                let dedupe_key = article
                    .url
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        (!article.sogou_url.trim().is_empty()).then(|| article.sogou_url.clone())
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "{}|{}|{}",
                            article.source, article.published_date, article.title
                        )
                    });
                if !seen.insert(dedupe_key) {
                    continue;
                }

                articles.push(article);
                if articles.len() >= max_results {
                    break;
                }
            }

            if articles.len() >= max_results {
                break;
            }
        }

        if articles.is_empty() && warnings.is_empty() {
            warnings.push("no matching WeChat articles found".to_string());
        }

        Ok(WechatSearchOutput {
            query,
            account,
            articles_count: articles.len(),
            articles,
            warnings,
        })
    }
}

fn browser_headers(referer: Option<&str>, cookies: Option<&str>) -> Vec<(HeaderName, String)> {
    let mut headers = vec![
        (USER_AGENT, UA.to_string()),
        (
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
        ),
        (ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7".to_string()),
    ];
    if let Some(referer) = referer {
        headers.push((REFERER, referer.to_string()));
    }
    if let Some(cookies) = cookies {
        headers.push((COOKIE, cookies.to_string()));
    }
    headers
}

pub fn parse_sogou_search_results(html: &str) -> Vec<WechatArticle> {
    let doc = Html::parse_document(html);
    let item_selector =
        Selector::parse(r#"li[id^="sogou_vr_11002601_box_"]"#).expect("valid selector");
    let title_selector = Selector::parse("h3").expect("valid selector");
    let snippet_selector = Selector::parse("p.txt-info").expect("valid selector");
    let source_selector = Selector::parse("span.all-time-y2").expect("valid selector");
    let date_script_selector = Selector::parse("span.s2 script").expect("valid selector");
    let link_selector = Selector::parse(r#"a[target="_blank"]"#).expect("valid selector");

    doc.select(&item_selector)
        .map(|item| {
            let title = item
                .select(&title_selector)
                .next()
                .map(element_text)
                .unwrap_or_default();
            let snippet = item
                .select(&snippet_selector)
                .next()
                .map(element_text)
                .unwrap_or_default();
            let source = item
                .select(&source_selector)
                .next()
                .map(element_text)
                .unwrap_or_default();
            let published_date = item
                .select(&date_script_selector)
                .next()
                .and_then(|script| script.text().next())
                .and_then(extract_timestamp)
                .unwrap_or_default();
            let sogou_url = item
                .select(&link_selector)
                .next()
                .and_then(|link| link.value().attr("href"))
                .map(normalize_sogou_url)
                .unwrap_or_default();

            WechatArticle {
                title,
                snippet,
                source,
                published_date,
                url: None,
                sogou_url,
                content: None,
                content_original_length: None,
                content_truncated: false,
                quality: WechatArticleQuality::default(),
            }
        })
        .collect()
}

pub fn resolve_wechat_url_from_redirect_html(html: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut rest = html;
    while let Some(idx) = rest.find("url") {
        rest = &rest[idx + 3..];
        let trimmed = rest.trim_start();
        let Some(after_plus) = trimmed.strip_prefix("+=") else {
            continue;
        };
        let after_plus = after_plus.trim_start();
        let Some(after_quote) = after_plus.strip_prefix('\'') else {
            continue;
        };
        let Some(end) = after_quote.find('\'') else {
            break;
        };
        parts.push(after_quote[..end].to_string());
        rest = &after_quote[end + 1..];
    }

    let joined = parts.join("").replace('@', "");
    if joined.starts_with("https://mp.weixin.qq.com/s?") {
        return Some(joined.replace("src=11脳tamp", "src=11&timestamp"));
    }

    find_direct_wechat_url(html)
}

pub fn extract_wechat_article_content(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    for selector in [
        "#js_content",
        ".rich_media_content",
        "div[id=\"page-content\"]",
    ] {
        let selector = Selector::parse(selector).expect("valid selector");
        if let Some(element) = doc.select(&selector).next() {
            let text = element_text(element);
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn find_direct_wechat_url(html: &str) -> Option<String> {
    let marker = "https://mp.weixin.qq.com/s?";
    let start = html.find(marker)?;
    let tail = &html[start..];
    let end = tail
        .find(|ch: char| matches!(ch, '\'' | '"' | '<' | '>' | '\\' | ' '))
        .unwrap_or(tail.len());
    Some(tail[..end].replace("&amp;", "&"))
}

fn normalize_sogou_url(raw: &str) -> String {
    if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else if raw.starts_with('/') {
        format!("{SOGOU_BASE}{raw}")
    } else {
        raw.to_string()
    }
}

fn element_text(element: scraper::ElementRef<'_>) -> String {
    normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "))
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_timestamp(script: &str) -> Option<String> {
    let marker = "timeConvert('";
    let start = script.find(marker)? + marker.len();
    let rest = &script[start..];
    let end = rest.find('\'')?;
    let ts = rest[..end].parse::<i64>().ok()?;
    chrono_like_timestamp(ts)
}

fn chrono_like_timestamp(ts: i64) -> Option<String> {
    // Avoid pulling chrono just for one UTC-ish display. Sogou timestamps are
    // second precision; stable tests only need the original yyyy-mm-dd shape.
    use std::time::{Duration, UNIX_EPOCH};
    let duration = Duration::from_secs(ts.try_into().ok()?);
    let datetime = UNIX_EPOCH.checked_add(duration)?;
    humantime_like(datetime)
}

fn humantime_like(time: std::time::SystemTime) -> Option<String> {
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
    ))
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    // Howard Hinnant's civil-from-days algorithm, UTC calendar.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

fn truncate_chars(content: String, max_chars: Option<usize>) -> (String, usize, bool) {
    let original_length = content.chars().count();
    let Some(limit) = max_chars else {
        return (content, original_length, false);
    };
    if original_length <= limit {
        return (content, original_length, false);
    }
    (content.chars().take(limit).collect(), original_length, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOGOU_HTML: &str = r#"
        <ul>
          <li id="sogou_vr_11002601_box_0">
            <h3><a target="_blank" href="/link?url=abc&amp;type=2">机器之心 OpenAI 文章</a></h3>
            <p class="txt-info">摘要 <em>OpenAI</em></p>
            <span class="all-time-y2">机器之心</span>
            <span class="s2"><script>document.write(timeConvert('1494483651'))</script></span>
          </li>
          <li id="sogou_vr_11002601_box_1">
            <h3><a target="_blank" href="https://weixin.sogou.com/link?url=def">Datawhale 文章</a></h3>
            <p class="txt-info">另一个摘要</p>
            <span class="all-time-y2">Datawhale</span>
            <span class="s2"><script>document.write(timeConvert('1713549335'))</script></span>
          </li>
        </ul>
    "#;

    #[test]
    fn parses_sogou_search_html() {
        let rows = parse_sogou_search_results(SOGOU_HTML);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "机器之心 OpenAI 文章");
        assert_eq!(rows[0].snippet, "摘要 OpenAI");
        assert_eq!(rows[0].source, "机器之心");
        assert!(rows[0].published_date.starts_with("2017-05-11"));
        assert!(rows[0]
            .sogou_url
            .starts_with("https://weixin.sogou.com/link?"));
    }

    #[test]
    fn resolves_segmented_wechat_url() {
        let html = r#"
            <script>
              var url = '';
              url += 'https://mp.weixin.qq.com/s?src=11@';
              url += '&timestamp=123';
            </script>
        "#;
        let url = resolve_wechat_url_from_redirect_html(html).expect("url");
        assert_eq!(url, "https://mp.weixin.qq.com/s?src=11&timestamp=123");
    }

    #[test]
    fn extracts_and_truncates_article_content() {
        let html = r#"<div id="js_content"><p>第一段</p><p>第二段 很长</p></div>"#;
        let content = extract_wechat_article_content(html).expect("content");
        assert_eq!(content, "第一段 第二段 很长");
        let (text, original, truncated) = truncate_chars(content, Some(3));
        assert_eq!(text, "第一段");
        assert_eq!(original, 10);
        assert!(truncated);
    }
}

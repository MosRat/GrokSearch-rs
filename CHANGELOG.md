# Changelog

All notable changes to GrokSearch-rs are documented here.

## Unreleased

## 0.3.2 - 2026-07-01

### Added

- Added MiniMax/Anthropic-compatible multimodal PDF artifact completion for
  `academic_pdf_artifacts`, including LLM-suggested figure/table bounding boxes,
  refined crops, reconstructed table Markdown, raw completion diagnostics, and
  validation warnings.
- Added pdf_oxide-backed page rendering and high-risk page triage for vision
  artifact analysis, plus redb-backed vision artifact caching.

### Changed

- Changed `academic_pdf_artifacts` vision handling to `vision_profile:"auto"`:
  LLM-refined artifact completion runs by default when an LLM API key is
  configured and otherwise stays off.
- Increased default vision artifact concurrency to 4 while keeping explicit
  per-call bounds.
- Updated repo-local academic and diagnostics skills with LLM-refined PDF
  artifact guidance.

## 0.3.1 - 2026-07-01

### Added

- Added intent-oriented academic PDF tools: `academic_pdf_read`,
  `academic_pdf_structure`, `academic_pdf_artifacts`, and
  `academic_pdf_download`, while keeping the legacy academic PDF commands
  compatible.
- Added LLM-assisted progressive paper reading for PDF structure extraction,
  backed by Anthropic-compatible clients, profile-based defaults, structured
  output, and local KV caching.
- Added redb-backed PDF byte caching with TTL, max-entry, max-size, refresh,
  bypass, and diagnostics support.
- Added global configuration for LLM and progressive/PDF cache defaults.

### Changed

- Refactored PDF parsing into pass-based pipelines for raw extraction, text
  cleaning, image/table artifacts, manifests, and final output truncation.
- Improved image and table artifact extraction with staged manifests, caption
  diagnostics, duplicate detection, table quality scoring, and markdown table
  fallback.
- Simplified agent-facing PDF workflows so reading text, generating structure,
  extracting artifacts, and downloading PDFs no longer depend on users calling
  another tool first.
- Optimized PDF downloads with retry backoff, elapsed-time diagnostics,
  host-aware adaptive range/full strategies, and direct-client fallback.

### Fixed

- Improved LLM progressive parsing resilience with bounded retries, JSON repair
  handling, nonfatal chunk failures, and safer local patch validation.

## 0.3.0 - 2026-06-29

### Added

- Added dedicated WeChat public-account article search via `wechat_search`,
  backed by Sogou Weixin recall with resolved `mp.weixin.qq.com` URLs and
  best-effort article content extraction.
- Added dedicated Zhihu OpenAPI site search via `zhihu_search`, including
  `zhihu_api_key` config support and compatible `ZHIHU_ACCESS_SECRET` /
  `ZHIHU_API_KEY` environment aliases.
- Added repository metadata lookup for GitHub and Hugging Face repositories,
  models, and datasets.
- Added academic PDF parse/download artifact workflows and PDF parser
  extraction support.
- Added repo-local GrokSearch skills covering web research, academic
  literature, social search, repository intelligence, and diagnostics.

### Changed

- Moved MCP tool descriptions and input schemas into embedded JSON specs so
  agent-facing tool documentation can be updated without editing Rust dispatch
  code.
- Refactored configuration into schema-driven modules. A single Rust schema now
  drives TOML/env mapping, alias precedence, redaction, generated config
  templates, and generated configuration reference documentation.
- Split configuration internals into focused model, reader, loader, paths,
  diagnostics, template, docs, and utility modules while preserving the public
  config API.

## 0.2.1 - 2026-06-28

### Fixed

- Publish PyPI Linux GNU/glibc wheel tags alongside musllinux tags while
  reusing the existing static musl binaries, so `pip` and `uv` can install the
  Linux wheels on mainstream GNU Linux distributions.

## 0.2.0 - 2026-06-27

### Changed

- Made HTTP client construction fallible across the shared net layer and
  provider wiring, so invalid timeout/proxy/TLS/cookie configuration now
  returns a configuration error instead of silently falling back to a default
  client.
- Added a hard `GROK_SEARCH_MAX_INLINE_SOURCES` limit of `20`; `0` now
  explicitly disables inline source content while keeping source metadata.
- Split the heaviest `SearchService` responsibilities into internal fetch,
  enrichment, and diagnostics modules while preserving public service and tool
  output shapes.

### Fixed

- Preserved institutional invalid-certificate route handling under the new
  fallible HTTP client builders.
- Reduced `ConfigLoadError` size so the workspace passes strict clippy checks.

## 0.1.29 - 2026-06-27

### Added

- Added an opt-in JSONL debug log (`GROK_SEARCH_DEBUG_LOG_PATH` /
  `debug_log_path`) with request/session IDs, operation timings, structured
  errors, and recursive redaction for secrets, tokens, cookies, and passwords.
- Added verbose `doctor` diagnostics for CLI and MCP, including response limits,
  debug log status, provider wiring, proxy details, and URL policy summaries.
- Added a global upstream response byte cap (`GROK_SEARCH_MAX_RESPONSE_BYTES` /
  `max_response_bytes`) wired through Grok/OpenAI-compatible, Tavily,
  Firecrawl, academic providers, content fetches, and SSE streams.

### Changed

- Restored TLS validation by default for IEEE/ACM institutional fallback routes;
  HTTP and invalid TLS are only allowed for private/local institutional targets.
- Split `SearchService` pure logic into domain filtering, response budgeting,
  and URL policy helpers while preserving public tool output shapes.
- Classified errors more precisely with structured `kind`, `retryable`, `hint`,
  and MCP `error.data` diagnostics.

### Fixed

- Rejected private, localhost, link-local, multicast, unspecified, and
  unique-local URLs in `web_fetch` and `web_map` before provider dispatch.
- Failed fast on malformed explicit config files instead of silently falling
  back to defaults.
- Limited `web_map.max_results` to `1..=50` at the tool boundary.
- Cleaned up local release/npm scratch artifacts from the working tree and
  ignored future `/dist/` and root `/package.json` scratch output.

## 0.1.28 - 2026-06-27

### Added

- Added an IEEE/ACM institutional PDF fallback for `academic_read`, with
  cookie-aware clients, direct/proxy route probing, PDF warmup requests, and
  `doctor` diagnostics for discovered institutional access.
- Added configuration toggles for the institutional fallback:
  `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ENABLED`,
  `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS`, and
  `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_PROBE`.

### Changed

- Prefer institutional full-text locations ahead of Sci-Hub when matching
  IEEE/ACM papers or direct IEEE/ACM URLs.
- Speed up release workflow builds with target-scoped Rust build caches and
  pip caching for Python wheel packaging.

## 0.1.27 - 2026-06-26

### Fixed

- Improved academic canonical resolution for title-like `academic_get` queries
  by preferring stronger Semantic Scholar, arXiv, and DBLP metadata over
  lower-confidence OpenAlex/Crossref fallbacks.
- Made `academic_citations` resolve papers canonically before fetching citation
  graphs, and removed noisy `openalex_reference` source entries from returned
  citation summaries.
- Made `academic_read` try multiple full-text PDF candidates before failing,
  so one timed-out or broken PDF URL does not end the read path.
- Rejected malformed `web_fetch` and `web_map` URLs at the service boundary with
  `invalid params` instead of forwarding them to Tavily or Firecrawl.

### Added

- Added a GitHub repository README specialist for `web_fetch`, returning clean
  repository metadata and README Markdown with `source_type: "github_repo"`.
- Expanded regression tests for academic canonical ranking, citation cleanup,
  URL validation, GitHub repo fallback behavior, README decoding, and source
  type serialization.

## 0.1.25 - 2026-06-26

### Fixed

- Added Semantic Scholar request throttling across endpoints, process-local and
  CLI process boundaries, plus conservative 429 backoff for the official 1 rps
  API key limit.
- Tightened academic search/get quality edges: `max_results=0` now returns an
  empty result, noisy arXiv matches are filtered, and title-like `academic get`
  rejects near-miss DBLP results.
- Improved academic PDF handling by ignoring empty PDF URLs, normalizing arXiv
  PDF links, retrying PDF downloads with a broader `Accept` header, and
  reporting clearer download/validation errors.
- Added GitHub token fallback through `gh auth token` when `GITHUB_TOKEN` is
  not configured, without leaking token values in diagnostics.

### Docs

- Documented Semantic Scholar `x-api-key` usage and throttling behavior.
- Documented GitHub CLI token fallback for GitHub issue/PR fetches.

## 0.1.17 - 2026-06-12

### Added

- **Grok Responses transport 兜底抽取内联 `[[n]](url)` 引用。** 经代理 /
  OpenAI 兼容网关转发的 Responses 端点常把真实搜索引用以内联 Markdown 写进
  回答正文而非结构化 citation 字段,此前这些 source 在 Responses transport
  下整体丢失。手写扫描器从 chat-completions 适配器的私有函数提升为
  `adapters::sources::extract_inline_bracket_citations` 共享函数(带
  provider 标签参数),两条 transport 复用;在结构化路径之后运行,
  `dedupe_sources` 将重复项折叠进更丰富的结构化条目。
- **Tavily 多 key 轮询。** `TAVILY_API_KEY` / `tavily_api_key` 接受逗号分隔的
  key 列表(`tvly-a,tvly-b`),单 key 配置零变化。多 key 时每个请求按共享
  原子游标 round-robin 选起始 key,把 credit 消耗均摊到所有 key;遇到
  key 维度错误(HTTP 401/403 无效鉴权、429 速率限制、432 套餐配额耗尽、
  433 PAYG 限额,均出自 Tavily 官方错误码表)自动换下一个 key 重试,每
  key 至多一次;超时/5xx 等上游故障不轮转,避免无谓加倍延迟。轮转时仅向
  stderr 打印 key 序号与状态码,绝不打印 key 本身。
- `http.rs` 新增 status-aware 变体 `post_json_with_status`(返回
  `HttpFailure { status, error }`),`post_json` 改为其丢弃 status 的薄
  包装,其余 provider 行为零变化。

### Docs

- README 补录 0.1.16 响应预算特性(Features 新增 bullet;配置表补
  `GROK_SEARCH_MAX_INLINE_SOURCES` / `GROK_SEARCH_RESPONSE_MAX_CHARS`;
  Tools 表补 `web_search` `response_format` 与 `get_sources`
  `offset`/`limit` 分页)。docs/CONFIGURATION.md 新增 Response budget
  小节与 TOML 键映射(`max_inline_sources` / `response_max_chars`)。

## 0.1.16 - 2026-06-11

### Added

- **`web_search` 响应预算,杜绝超大返回打爆 MCP 客户端 25k token 上限。**
  对齐 Tavily/Exa/官方 fetch server 的"默认小响应 + 显式截断提示 + drill-down
  通道"模式:
  - 内联正文只填充前 `max_inline_sources` 个 source(默认 5,env
    `GROK_SEARCH_MAX_INLINE_SOURCES`);其余 source 仅返回元数据,需要原文时用
    `web_fetch(url)` 跟进。此前 Grok 返回 20+ 引用时会全部内联(每条最多
    15k 字符),单次响应可超 300k 字符。
  - 整响应字符预算 `response_max_chars`(默认 60000,env
    `GROK_SEARCH_RESPONSE_MAX_CHARS`),按真实载荷计权(答案 + 各 source 的
    元数据与内联正文):超预算时先从尾部 source 截断/省略正文,被截断的
    source 携带指引注记(`web_fetch(...)` / `get_sources(...)`);元数据本身
    溢出时(宽调研 query 下 Grok 可返回 50+ 引用)继续从尾部丢弃整条
    source(至少保留 1 条),输出新增 `truncated` 字段。会话缓存始终保留
    全量内容,按需取回不丢失。实测原 317k 字符的响应降至 ~62k。
  - `get_sources` 支持 `offset`/`limit` 分页(借鉴官方 fetch server 的
    `start_index` 续读模式),输出新增 `total_sources`、`offset`、
    `next_offset`、`truncated` 字段。
  - `web_search` 新增 `response_format`(`"concise"` | `"detailed"`,
    Anthropic 工具设计指南推荐的枚举):`concise` 只返回综合答案 + source
    元数据;显式传入时优先于遗留的 `include_content`。非法取值报
    `invalid_params`。

## 0.1.15 - 2026-06-08

### Changed

- **`web_search` 降级原因细分。** Grok 调用失败时 `fallback_reason` 不再一律
  归为 `grok_provider_error`,而是按错误类型区分:`grok_timeout`(超时)、
  `grok_auth_error`(鉴权)、`grok_parse_error`(响应解析失败),其余仍为
  `grok_provider_error`,便于排查降级到底是限流/超时还是上游故障。

## 0.1.14 - 2026-06-05

### Added

- **Source extraction pipeline for `web_fetch`.** GitHub issues/PRs,
  StackExchange/MathOverflow questions, arXiv abstracts, and Wikipedia articles
  now route through specialist extractors that return clean, structured Markdown
  (titles, state/labels, accepted-answer ordering, abstracts, vote-sorted
  answers) instead of a generic page scrape. Non-matching URLs fall back to the
  existing Tavily → Firecrawl chain. `web_fetch` output gains two fields:
  `source_type` (`github_issue` | `github_pull` | `stackexchange` | `arxiv` |
  `wikipedia` | `generic`) and `fallback_reason` (set only when a matched
  specialist failed and the generic path was used).
- **`web_search` 内联富化(opt-in `include_content`)。** 开启后,对返回的 top
  sources 复用同一套提取管道补全正文,受 `enrich_concurrency` 与全局 deadline
  约束,保持"每次 web_search 仅一次源 provider 调用"契约不变。`Source` 新增
  `content` 字段(`skip_serializing_if` 缺省不输出)。
- **新配置项:** `GROK_SEARCH_SOURCE_MAX_ANSWERS` / `GROK_SEARCH_SOURCE_MAX_COMMENTS`
  (StackExchange/GitHub 折叠上限)、`GITHUB_TOKEN`(鉴权抓取,提升 GitHub API
  限额)、`GROK_SEARCH_ENRICH_CONCURRENCY`(并发富化,clamp 1..5)、
  `GROK_SEARCH_ENRICH_MAX_CHARS`。
- `doctor` 与 `redacted_diagnostics` 报告 `github_token` 的 `set | unset` 状态。

### Fixed

- **StackExchange / MathOverflow `web_fetch` 返回原始 HTML 而非 Markdown。** 提取器
  优先读 `body_markdown`,但 API 用 `filter=withbody` 调用,只会返回渲染后的 HTML
  `body`,markdown 分支从未触发。改用确定性自定义 filter(`base=withbody` +
  `question/answer.body_markdown`),`<p>` / `<pre><code>` 结构性 HTML 消除。
- **StackExchange `body_markdown` 携带 HTML 实体**(`&lt;` `&gt;` `&quot;`
  `&#39;` `&#233;`),污染代码块(`c &lt; arraySize`)与正文(`Poincar&#233;`)。
  新增纯函数 `decode_entities`(named `lt/gt/amp/quot/apos` + 十进制/十六进制
  数值引用),在 body / title / 作者名处解码;未知实体与裸 `&` 原样保留。
- hand-written `Config` `Debug` 实现对密钥脱敏。

### Performance

- **StackExchange 的 question 与 answers 两个端点并发抓取(`tokio::join!`)。**
  二者无数据依赖(answers URL 仅由 `id`/`site` 构造),改并发后实测
  StackExchange 614ms → 369ms(−40%)、MathOverflow 578ms → 294ms(−49%);
  question 仍强制、answers 仍 best-effort,行为不变。

### Internal

- 新增 `sources` 模块:`SourceExtractor` trait、`SourceRouter`(有序首匹配)、
  统一入口 `resolve_content`。
- 新增依赖:`quick-xml`(arXiv Atom 解析,不解析 DTD/外部实体,无 XXE 风险)、
  `percent-encoding`(Wikipedia 标题安全编码);`tokio` 启用 `time` feature
  支撑富化 deadline。
- 五个提取器均带离线 fixture 单测;SE 端点 URL 构造抽为纯函数并钉死
  `body_markdown` filter,防止 HTML 回归。

### Notes

- 部分 StackExchange 答案的 `body_markdown` 含作者手写的内联 HTML(`<a>`、
  `<br>`、`<blockquote>` 等)——SE markdown 语法本就允许,按忠实原文保留。
- 0.1.12 / 0.1.13(Grok OAuth 登录模式、SSE 帧解析加固)此前已发布 tag 但未在本
  文档登记;此条目不追溯补录。

## 0.1.11 - 2026-05-16

### Added

- OpenAI-compatible chat/completions transport (`OPENAI_COMPATIBLE_API_URL` / `_KEY` / `_MODEL`). When `GROK_SEARCH_API_KEY` is unset and the new triple is set, the client talks to `/v1/chat/completions` instead of `/v1/responses`. Source extraction covers OpenAI annotations, Perplexity-style citations, top-level `search_sources`, and inline `[[n]](url)` markers.
- `Config::transport` enum (`Responses` | `ChatCompletions`) decided at startup from the configured env-var groups.
- 跨平台配置路径解析：Windows（PowerShell / cmd）下自动回退到 `%USERPROFILE%\.config\grok-search-rs\config.toml`，无需再手动设置 `$env:HOME`。Unix / macOS / Git Bash 沿用 `$HOME/.config/...`，行为零变化。`grok-search-rs --init` 在三平台均一键直跑。
- `config::config_path_for(env_map)` 公共测试入口，便于注入式断言跨平台路径解析。

### Fixed

- `SearchService::build_search_request` 过去把 `config.grok_model` 硬塞进每个 `SearchRequest`，导致 ChatCompletions transport 下 `OPENAI_COMPATIBLE_MODEL` 被悄悄屏蔽——网关收到的是 Grok 专属模型 ID。`doctor()` 也同病，硬编 `"grok_responses"` 与 `grok_model`。现改为构造期由 `config.transport` 一次性解析 `default_model`（Responses → `grok_model`；ChatCompletions → `openai_compatible_model`，缺省回落 `grok_model`），`build_search_request` / `probe_grok` / `doctor()` 统一复用；`WebSearchInput.model` 显式覆盖仍优先。`doctor()` 同步按 transport 返回真实 `api_url` 与强制 `x_search_enabled=false`（该 flag 在 chat 路径本就被忽略）。
- Windows 下 `grok-search-rs --init` 报错 `cannot resolve config path: HOME is unset...` —— 根因是 PowerShell/cmd 默认无 `HOME` 变量，现已改为优先 `HOME`、回退 `USERPROFILE`，错误信息同步更新。

### Notes

- `x_search_enabled` is silently ignored on the Chat-Completions transport (warned once at startup).
- No new crates added.

## 0.1.10 - 2026-05-16

### Fixed

- `web_search` 在指向 OpenAI-Responses 兼容网关（LiteLLM / OneAPI / vLLM 等）时，因上游默认返回 SSE 流（`Content-Type: text/event-stream`，首字节 `e`），`post_json` 的 `serde_json::from_slice` 抛出 `invalid Grok Responses JSON: expected value at line 1 column 1`，导致 Grok 通道整体失败、回退到 Tavily-only 结果。修复：`to_grok_responses_payload` 在请求体显式声明 `"stream": false`，迫使兼容网关返回同步 JSON。xAI 官方 `/v1/responses` 不识别该字段会静默忽略，对官方端点零影响。感谢 @bigsuperangel（#1）。

## 0.1.9 - 2026-05-16

### Added

- 全局配置文件支持：`~/.config/grok-search-rs/config.toml` 一次设定，所有 MCP 客户端共享，免去逐个 client 重复填 `env`。路径可由 `GROK_SEARCH_CONFIG=/abs/path` 覆盖。
- 优先级链：**进程 env > 配置文件 > 内置默认**，per-client `env` 仍可临时覆写文件设置。
- 配置文件支持全部 16 个键（`grok_api_key` / `grok_model` / `tavily_api_key` / `firecrawl_api_key` / `default_extra_sources` / `timeout_seconds` / …），键名为对应 env 变量的 snake_case 形式；未知键会被拒绝，杜绝拼写错误静默丢失。
- 新增 `grok-search-rs --init` 子命令：幂等地写入带注释的模板到解析后的配置路径，所有键默认注释掉——空模板与"无配置文件"等价，绝不静默覆盖默认值。
- 交互式 onboarding（直接执行二进制时）：检测到配置文件缺失时，自动追加 `--init` 提示。
- 新增 `Config::load_from(env_map)`、`config::config_path()`、`config::write_template(path)` 公共 API。

### Internal

- 新增 `toml = "0.8"` 依赖（仅 `parse` feature，约 4 个传递 crate）。
- 配置加载新增 6 条集成测试：文件供值、env 覆盖、文件缺失、全键映射、`--init` 幂等性、空模板不改默认行为。

### Performance

- `web_search` 改为投机并发：`tokio::join!` 同时发起 Grok 与 Tavily 检索，总延迟由 `sum(Grok, Tavily)` 降为 `≈ max(Grok, Tavily)`；通过 `count = max(extra_sources, fallback_sources)` 一次取够、按路径裁剪复用，**保持"每次 web_search 仅一次源 provider 调用"契约**。
- 三家 provider（Grok / Tavily / Firecrawl）共享单个 `reqwest::Client`，启用 `gzip`、`pool_idle_timeout=90s`、`tcp_keepalive=60s`、`tcp_nodelay`；TLS 会话与连接池跨 host 复用。
- HTTP 响应解析切 `bytes()` + `serde_json::from_slice`，省去一次完整 UTF-8 校验扫描。
- `apply_fetch_limit` 改单次 `char_indices` 截断，UTF-8 文本由三遍扫描压到一遍。
- `Source.provider` 字段由 `String` 切 `Cow<'static, str>`：所有内部标签都是 `'static`，逐源省去一次堆分配。
- `SourceCache` 内部存 `Arc<Vec<Source>>`：cache get/set 在 mutex 内仅做引用计数，临界区由 O(N) 深拷降为 O(1)。
- Tavily search 请求体改 serde 派生结构体：消除 `json! + as_object_mut + insert` 多次临时 String 分配。
- `session_id` 编码改栈缓冲（`uuid::fmt::Simple::encode_lower` 写 `[u8; 32]`），省两次 String 分配。

### Changed

- tokio runtime flavor 由 `multi_thread` 切 `current_thread`：MCP stdio 服务本就单流，去掉 N 个 worker 线程降低稳态 RSS（预期 0.3~0.8 MB）。
- `[profile.release]` 启用 `panic = "abort"`：移除 unwind 表，release 二进制由 3.0 MB 降至 **2.5 MB（−16.6%）**。
- `reqwest` 启用 `gzip` feature。

### Internal

- 新增 `GrokResponsesProvider::with_client` / `TavilyProvider::with_client` / `FirecrawlProvider::with_client` 构造路径，旧 `new(.., timeout)` 签名保留以兼容下游集成。
- 新增 `RawSourceOrigin` 枚举与 `enrichment_label` / `fallback_label` 自由函数，把"取源"与"打路径标签"解耦。
- 测试新增 3 条契约：`get_sources_returns_same_payload_repeatedly`、`web_search_speculation_serves_enrichment_with_one_source_call`、`source_provider_field_accepts_static_str_via_cow`。

### Verified

- `cargo test --all`：34 passed / 0 failed
- `cargo clippy --all-targets -- -D warnings`：零警告
- MCP stdio 烟测：`initialize` + `tools/list` 协议握手通过，五工具齐全
- 所有公共 MCP tool 输入 / 输出 schema **零变更**

## 0.1.7 - 2026-05-15

### Added

- `web_search` 新增 `recency_days` / `include_domains` / `exclude_domains` 输入参数：Tavily 端真过滤（`days` + `topic=news` 及 include/exclude 域名），Grok 端以软提示形式注入 prompt。
- `web_fetch` 新增 `max_chars` 输入参数与 `GROK_SEARCH_FETCH_MAX_CHARS` 环境兜底；返回结构扩展为 `{url, content, original_length, truncated}`，便于 LLM 感知截断。

### Changed

- `web_search` 输出回炉：撤掉懒加载契约与 `sources_preview` 字段，改为常驻 `sources: [...]`——成功路径返回 Grok 原生 + Tavily 补强 merge 后的完整列表；fallback 路径返回 Tavily 兜底的完整列表。每条含 `{url, provider, title?, description?, published_date?}`。`session_id` 与 `get_sources` 保留作缓存回查入口，但不再是获取首次响应来源的必经路径。
- `GROK_SEARCH_EXTRA_SOURCES` 默认值由 `0` 调整为 `3`，使开放检索默认即享 Tavily 补强；如需关闭显式设 `0`。
- `SourceProvider::search_sources` trait 签名扩展接收 `&SearchFilters`，Tavily 透传，Firecrawl 忽略（无对应能力）。

## 0.1.6 - 2026-05-15

### Fixed

- `doctor` 的 Grok 探针现在携带 `web_search` tool intent，避免上游误判为 parse error 导致 `reachable=false` 与实际可用状态不符。

### Changed

- 默认 `GROK_SEARCH_MODEL` 由 `grok-4.3` 调整为 `grok-4-1-fast-reasoning`（同步 README、`.env.example`、docs/CONFIGURATION.md、tests/config.rs）。
- `web_map` 输出裁剪为仅 `{url, provider}`，剥离对地图发现场景无用的 `title` / `description` / `published_date`，减小响应体。
- 抽出 `src/providers/http.rs` 公共 `build_client` 与 `post_json`，三个 provider（Grok / Tavily / Firecrawl）共享同一份 reqwest client 构造与 HTTP 错误归类逻辑。
- 合并测试用 4 个 `fake_with_*` 工厂方法为 `fake_with_sources` + `fake_custom`，净减约 70 行测试样板。
- README Tools 表与 docs/TESTING.md 清理 0.1.5 已下线的工具与测试条目，与当前 5 件 MCP 工具表面对齐。

### Removed

- 本地 `GrokSearch-rs-rebuild-plan.md` 历史规划稿（原本即在 `.gitignore` 内）。

## 0.1.5 - 2026-05-15

### Removed

- Planning compatibility tools (`plan_intent`, `plan_search`, `plan_search_term`, `plan_sub_query`, `plan_tool_mapping`, `plan_execution`, `plan_complexity`) and their tests.
- Built-in tool toggle support (`toggle_builtin_tools`) and its test.
- Auxiliary tools `health`, `get_config_info`, `switch_model` from the MCP surface.

### Changed

- Reduced MCP surface to 5 tools: `web_search`, `get_sources`, `web_fetch`, `web_map`, `doctor`.
- Replaced ad-hoc health/config probes with a single `doctor` diagnostic that performs live connectivity checks against Grok, Tavily, and Firecrawl and returns masked configuration.
- Tightened provider modules (`grok`, `tavily`, `firecrawl`) and simplified `SearchService` source caching.

### Added

- Tag-driven release pipeline: pushing `vX.Y.Z` builds binaries, publishes 6 npm packages, and syncs `Cargo.toml` / `Cargo.lock` / all `package.json` files back to `main` automatically.
- Manual fallback `scripts/bump-version.sh` and `Bump Version` GitHub Actions workflow.

## 0.1.4 - 2026-05-15

### Fixed

- Ignored JSON-RPC notifications such as `notifications/initialized` instead of emitting `id: null` error responses during MCP startup.
- Added MCP `ping` request support.

## 0.1.3 - 2026-05-15

### Fixed

- Aligned the npm launcher with `ace-tool-rs` by resolving the installed platform package directly and removing runtime GitHub release download fallback from MCP startup.

## 0.1.0 - 2026-05-14

### Added

- Rust MCP stdio server for Grok Responses-backed web search, Tavily source retrieval, and Firecrawl fallback.
- Single Grok Responses provider using `/v1/responses` with `web_search` enabled by default and optional `x_search`.
- `GROK_SEARCH_URL` normalization from root URL, `/v1` base URL, or endpoint-like URL to a `/v1` base.
- Tavily search fallback when Grok returns empty content, no verifiable sources, or provider errors.
- Tavily Extract-backed `web_fetch` and Tavily Map-backed `web_map`.
- Firecrawl-backed `web_fetch` fallback and supplemental source fallback.
- Source cache keyed by `session_id` and `get_sources` retrieval.
- Planning compatibility tools and built-in tool toggle support for Claude, Codex, and Gemini contexts.
- Regression coverage for provider payload shape, fallback behavior, Tavily parsing, source merging, planning, logging, and toggle aliases.

### Changed

- Public AI configuration now uses `GROK_SEARCH_API_KEY`, `GROK_SEARCH_URL`, and `GROK_SEARCH_MODEL`.
- `GROK_SEARCH_WEB_SEARCH` defaults to enabled.
- `GROK_SEARCH_X_SEARCH` defaults to disabled and must be explicitly enabled.

### Fixed

- Prevented the original GrokSearch issue #41 class of failure by ensuring Responses payloads include the intended web search tool.
- Treated empty or source-less Grok responses as unverifiable and routed them to source fallback.

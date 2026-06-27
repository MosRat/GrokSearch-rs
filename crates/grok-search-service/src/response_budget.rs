use grok_search_types::model::source::Source;

pub(crate) fn apply_response_budget(
    answer_chars: usize,
    sources: &mut Vec<Source>,
    budget: usize,
    session_id: &str,
) -> bool {
    let content_chars = |s: &Source| s.content.as_deref().map(|c| c.chars().count()).unwrap_or(0);
    let mut total: usize = answer_chars + sources.iter().map(source_weight).sum::<usize>();
    if total <= budget {
        return false;
    }

    for idx in (0..sources.len()).rev() {
        if total <= budget {
            break;
        }
        let len = content_chars(&sources[idx]);
        if len == 0 {
            continue;
        }
        let url = sources[idx].url.clone();
        let note = |verb: &str| {
            format!(
                "_[{verb}: response budget reached - full text via web_fetch(\"{url}\") or get_sources(session_id=\"{session_id}\", offset={idx}, limit=1)]_"
            )
        };
        let omit_note = note("inline content omitted");
        let omit_len = omit_note.chars().count();
        if len <= omit_len {
            continue;
        }
        let overshoot = total - budget;
        let trim_note = note("truncated");
        let trim_overhead = trim_note.chars().count() + 2;
        if len > overshoot + trim_overhead {
            let keep = len - overshoot - trim_overhead;
            let prefix: String = sources[idx]
                .content
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(keep)
                .collect();
            sources[idx].content = Some(format!("{prefix}\n\n{trim_note}"));
            total -= overshoot;
        } else {
            sources[idx].content = Some(omit_note);
            total = total - len + omit_len;
        }
    }

    while total > budget && sources.len() > 1 {
        let dropped = sources.pop().expect("len > 1");
        total -= source_weight(&dropped);
    }

    true
}

fn source_weight(source: &Source) -> usize {
    const JSON_OVERHEAD: usize = 64;
    let opt_chars = |v: &Option<String>| v.as_deref().map(|s| s.chars().count()).unwrap_or(0);
    source.url.chars().count()
        + source.provider.chars().count()
        + opt_chars(&source.title)
        + opt_chars(&source.description)
        + opt_chars(&source.published_date)
        + source
            .content
            .as_deref()
            .map(|c| c.chars().count())
            .unwrap_or(0)
        + JSON_OVERHEAD
}

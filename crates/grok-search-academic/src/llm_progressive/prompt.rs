use super::chunking::Chunk;

pub(crate) fn chunk_prompt(chunk: &Chunk, prompt_profile: &str) -> String {
    let anchors = chunk
        .anchors
        .iter()
        .take(160)
        .map(|anchor| {
            format!(
                "{} L{}: {}",
                anchor.anchor_id, anchor.line_start, anchor.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"You are cleaning and indexing extracted academic PDF text.
Profile: {prompt_profile}
Return ONLY valid JSON with this shape:
{{"patches":[{{"kind":"join_lines|dehyphenate|delete_noise_line|replace_small_span|mark_boundary","anchor_id":"...","original_excerpt":"...","replacement":null,"confidence":0.0,"reason":"..."}}],"section_candidates":[{{"title":"...","level":1,"start_anchor":"...","end_anchor":null,"confidence":0.0}}],"local_digest":{{"text":"80-160 words, evidence-only","anchors":["..."]}},"entities":[{{"kind":"figure|table|reference|resource","label":"...","anchor_id":"...","text":"...","confidence":0.0}}],"warnings":[]}}
Rules:
- Do not rewrite the whole chunk.
- Do not invent facts beyond cited anchors.
- Prefer omission over guessing.
- No markdown fences.

ANCHORS:
{anchors}

CHUNK TEXT:
<<<
{}
>>>"#,
        chunk.text
    )
}

pub(crate) fn repair_prompt(output: &str, error: &str) -> String {
    format!(
        r#"Repair this model output into valid JSON only.
Schema keys: patches, section_candidates, local_digest, entities, warnings.
Parse error: {error}

OUTPUT:
<<<
{output}
>>>"#
    )
}

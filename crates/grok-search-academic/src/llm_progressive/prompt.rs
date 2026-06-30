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
    match prompt_profile {
        "compact_v2" => compact_v2_chunk_prompt(chunk, &anchors),
        "compact" => compact_chunk_prompt(chunk, &anchors),
        "micro" => micro_chunk_prompt(chunk, &anchors),
        _ => verbose_chunk_prompt(chunk, prompt_profile, &anchors),
    }
}

fn verbose_chunk_prompt(chunk: &Chunk, prompt_profile: &str, anchors: &str) -> String {
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

fn compact_chunk_prompt(chunk: &Chunk, anchors: &str) -> String {
    format!(
        r#"Index academic PDF text. Output ONLY one minified JSON object with short keys.
Schema: {{"s":[{{"t":"section title","l":1,"a":"anchor","c":0.8}}],"d":"<=45 words, evidence-only","da":["anchor"],"e":[{{"k":"figure|table|reference|resource","x":"label/text","a":"anchor","c":0.8}}],"p":[{{"k":"dehyphenate|join_lines|delete_noise_line|replace_small_span|mark_boundary","a":"anchor","o":"exact original excerpt","r":"replacement or null","c":0.8}}],"w":[]}}
Hard limits: s<=5, e<=8, p<=2, da<=4. Prefer empty arrays over guessing. Use c>=0.55 only when confident; otherwise omit the item. Patches must be tiny local fixes that preserve the same letters/digits, only changing whitespace, hyphens, or punctuation. Do not expand or reconstruct missing words. No markdown. No extra keys.

ANCHORS:
{anchors}

TEXT:
<<<
{}
>>>"#,
        chunk.text
    )
}

fn compact_v2_chunk_prompt(chunk: &Chunk, anchors: &str) -> String {
    format!(
        r#"Index academic PDF text for a progressive reading structure. Output ONLY one minified JSON object with short keys.
Schema: {{"s":[{{"t":"true section heading","l":1,"a":"anchor","c":0.8}}],"d":"<=38 words, evidence-only","da":["anchor"],"e":[{{"k":"figure|table|reference|resource","x":"label/text","a":"anchor","c":0.8}}],"p":[{{"k":"dehyphenate|join_lines|delete_noise_line|replace_small_span|mark_boundary","a":"anchor","o":"exact original excerpt","r":"replacement or null","c":0.8}}],"w":[]}}
Hard limits: s<=3, e<=6, p<=1, da<=3. Prefer empty arrays over guessing.
Section rules: only mark real paper headings such as Abstract, Introduction, Methods, Results, References, Appendix, or numbered headings. Do NOT mark figure captions, table rows, author names, affiliations, footers, bullets, equations, or reference entries as sections.
Entity rules: capture figure/table captions, bibliography/reference entries, datasets/code/URL resources, but only when an anchor line explicitly contains the evidence.
Patch rules: patches are optional and tiny. They must preserve the exact same letters and digits, changing only whitespace, soft hyphens, line-break hyphens, or punctuation. Do not reconstruct missing text, expand abbreviations, or rewrite prose.
Digest rules: summarize only the local chunk, cite anchors in da, no global paper claims unless present in this chunk.
Use c>=0.6 only when confident. No markdown. No extra keys.

ANCHORS:
{anchors}

TEXT:
<<<
{}
>>>"#,
        chunk.text
    )
}

fn micro_chunk_prompt(chunk: &Chunk, anchors: &str) -> String {
    format!(
        r#"Index academic PDF text. Output ONLY one minified JSON object.
Schema keys:
- s: section tuples [title,level,anchor,confidence]
- d: digest tuple ["<=30 words",anchor1,anchor2]
- e: entity tuples [kind,label,anchor,confidence], kind is figure|table|reference|resource
- p: patch tuples [kind,anchor,exact_original,replacement_or_null,confidence]
- w: warning strings
Example: {{"s":[["Introduction",1,"c0000_x",0.8]],"d":["paper states the method goal","c0000_x"],"e":[],"p":[],"w":[]}}
Hard limits: s<=3, e<=5, p<=1. Patches must preserve the same letters/digits, only changing whitespace, hyphens, or punctuation. Prefer empty arrays. No markdown. No extra keys.

ANCHORS:
{anchors}

TEXT:
<<<
{}
>>>"#,
        chunk.text
    )
}

pub(crate) fn repair_prompt(output: &str, error: &str) -> String {
    format!(
        r#"Repair this model output into valid JSON only.
Accepted schemas:
1. compact: {{"s":[],"d":"","da":[],"e":[],"p":[],"w":[]}}
2. verbose: {{"patches":[],"section_candidates":[],"local_digest":{{"text":"","anchors":[]}},"entities":[],"warnings":[]}}
Parse error: {error}

OUTPUT:
<<<
{output}
>>>"#
    )
}

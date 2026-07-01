use grok_search_pdf::{PdfRenderedPage, PdfVisionPage};

pub(super) fn page_prompt(page: &PdfVisionPage, rendered: &PdfRenderedPage) -> String {
    let anchors = page
        .anchors
        .iter()
        .take(48)
        .map(|anchor| format!("{}: {}", anchor.anchor_id, anchor.excerpt))
        .collect::<Vec<_>>()
        .join("\n");
    let schema = serde_json::json!({
        "page_number": page.page_number,
        "figure_completions": [{
            "label": "Figure 1",
            "caption": "caption text visible or supplied",
            "bbox_norm": [0.12, 0.18, 0.84, 0.52],
            "caption_bbox_norm": [0.12, 0.53, 0.84, 0.58],
            "status": "crop_ready|fragmented|uncertain|not_visible",
            "confidence": 0.0,
            "notes": "short diagnostic"
        }],
        "table_completions": [{
            "label": "Table 1",
            "caption": "caption text visible or supplied",
            "bbox_norm": [0.08, 0.20, 0.92, 0.78],
            "status": "reconstructed|partial|uncertain|not_visible",
            "headers": ["column"],
            "rows": [["visible cell"]],
            "markdown": "| column |\\n| --- |\\n| visible cell |",
            "confidence": 0.0,
            "notes": "short diagnostic"
        }],
        "layout_warnings": [],
        "confidence": 0.0,
        "warnings": []
    });
    format!(
        "Complete missed academic PDF artifacts from a rendered page image and local pdf_oxide signals.\n\
         Page: {}\n\
         Render: {}x{} PNG at {} DPI\n\
         Triage priority: {}\n\
         Triage reasons: {}\n\
         Local anchors:\n{}\n\n\
         Return ONLY strict minified JSON matching this schema: {}\n\
         Coordinate rule: bbox_norm is [x0,y0,x1,y1] normalized to the rendered page image, top-left origin, each value 0..1.\n\
         Figure rule: bbox_norm covers ONLY the visual object, not caption or body paragraph. Include full plotted area, axes, labels, legend, all panels, and needed whitespace. Return caption_bbox_norm separately when visible. If caption exists but object is not visible, status=not_visible and bbox_norm=[].\n\
         Table rule: reconstruct only visible and legible cells. If the whole visible table is small, return all rows. If dense or partly unreadable, return headers plus visible sample rows and status=partial. Never invent rows, metrics, or labels.\n\
         Do not return body-text patches or prose summaries. Prefer omission over guessing. Keep notes short.",
        page.page_number,
        rendered.width,
        rendered.height,
        rendered.dpi,
        page.triage_priority,
        page.triage_reasons.join(", "),
        anchors,
        schema
    )
}

pub(super) fn system_prompt() -> &'static str {
    "You are a precise PDF artifact inspector. Use the image and anchors only. \
     Output JSON only. Do not infer unsupported facts. Do not rewrite article text."
}

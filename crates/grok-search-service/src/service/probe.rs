pub(crate) struct Probe {
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

impl Probe {
    pub(crate) fn ok(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
        }
    }
    pub(crate) fn failed(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
    pub(crate) fn skipped(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
}

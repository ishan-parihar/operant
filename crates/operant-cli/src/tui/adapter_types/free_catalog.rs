// adapter_types/free_catalog.rs — Free AI provider catalog.

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "catalog data: note is kept for the free-mode dialog"
)]
pub struct FreeUpstream {
    pub id: &'static str,
    pub title: &'static str,
    pub note: &'static str,
    pub key_url: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream {
        id: "groq",
        title: "Groq",
        note: "Blazing fast inference",
        key_url: "console.groq.com",
    },
    FreeUpstream {
        id: "cerebras",
        title: "Cerebras",
        note: "Ultra-fast wafer-scale",
        key_url: "cloud.cerebras.ai",
    },
    FreeUpstream {
        id: "google",
        title: "Google Gemini",
        note: "Multimodal, generous free tier",
        key_url: "aistudio.google.com",
    },
    FreeUpstream {
        id: "mistral",
        title: "Mistral",
        note: "Strong coding models",
        key_url: "console.mistral.ai",
    },
    FreeUpstream {
        id: "sambanova",
        title: "SambaNova",
        note: "Fast inference, free tier",
        key_url: "cloud.sambanova.ai",
    },
];

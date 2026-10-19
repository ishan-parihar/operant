// adapter_types/free_catalog.rs — Free AI provider catalog.

#[derive(Debug)]
pub struct FreeUpstream {
    pub id: &'static str,
    pub title: &'static str,
    pub key_url: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream {
        id: "groq",
        title: "Groq",
        key_url: "console.groq.com",
    },
    FreeUpstream {
        id: "cerebras",
        title: "Cerebras",
        key_url: "cloud.cerebras.ai",
    },
    FreeUpstream {
        id: "google",
        title: "Google Gemini",
        key_url: "aistudio.google.com",
    },
    FreeUpstream {
        id: "mistral",
        title: "Mistral",
        key_url: "console.mistral.ai",
    },
    FreeUpstream {
        id: "sambanova",
        title: "SambaNova",
        key_url: "cloud.sambanova.ai",
    },
];

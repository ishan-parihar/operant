// adapter_types/free_catalog.rs — Free AI provider catalog.

#[derive(Debug)]
pub struct FreeUpstream {
    pub id: &'static str,
    pub title: &'static str,
    pub default_model: &'static str,
    pub note: &'static str,
    pub key_url: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream {
        id: "groq",
        title: "Groq",
        default_model: "llama-3.3-70b-versatile",
        note: "Blazing fast inference",
        key_url: "console.groq.com",
    },
    FreeUpstream {
        id: "cerebras",
        title: "Cerebras",
        default_model: "llama-3.3-70b",
        note: "Ultra-fast wafer-scale",
        key_url: "cloud.cerebras.ai",
    },
    FreeUpstream {
        id: "google",
        title: "Google Gemini",
        default_model: "gemini-2.0-flash",
        note: "Multimodal, generous free tier",
        key_url: "aistudio.google.com",
    },
    FreeUpstream {
        id: "mistral",
        title: "Mistral",
        default_model: "mistral-small-latest",
        note: "Strong coding models",
        key_url: "console.mistral.ai",
    },
    FreeUpstream {
        id: "sambanova",
        title: "SambaNova",
        default_model: "Meta-Llama-3.3-70B-Instruct",
        note: "Fast inference, free tier",
        key_url: "cloud.sambanova.ai",
    },
];

pub(crate) fn reverse_provider_lookup(dev_provider: &str) -> String {
    for provider in crate::provider::PROVIDERS {
        if let Some(mapped) = operant_core::models_dev::provider_to_models_dev(provider.name)
            && mapped == dev_provider
        {
            return provider.name.to_string();
        }
    }
    dev_provider.to_string()
}

// ---------- ModelRegistry ----------

//! Provider alias functions used by config validation.
//!
//! These are extracted from the providers module to break the circular
//! dependency between config and providers.

/// `true` when `name` is a global GLM / Zhipu alias (`glm`, `zhipu`, ...).
pub fn is_glm_global_alias(name: &str) -> bool {
    matches!(name, "glm" | "zhipu" | "glm-global" | "zhipu-global")
}

/// `true` when `name` is a China-region GLM / Zhipu alias (`glm-cn`, `bigmodel`, ...).
pub fn is_glm_cn_alias(name: &str) -> bool {
    matches!(name, "glm-cn" | "zhipu-cn" | "bigmodel")
}

/// `true` when `name` is any GLM alias (global or China-region).
pub fn is_glm_alias(name: &str) -> bool {
    is_glm_global_alias(name) || is_glm_cn_alias(name)
}

/// `true` when `name` is a global Z.ai alias (`zai`, `z.ai`, ...).
pub fn is_zai_global_alias(name: &str) -> bool {
    matches!(name, "zai" | "z.ai" | "zai-global" | "z.ai-global")
}

/// `true` when `name` is a China-region Z.ai alias (`zai-cn`, `z.ai-cn`).
pub fn is_zai_cn_alias(name: &str) -> bool {
    matches!(name, "zai-cn" | "z.ai-cn")
}

/// `true` when `name` is any Z.ai alias (global or China-region).
pub fn is_zai_alias(name: &str) -> bool {
    is_zai_global_alias(name) || is_zai_cn_alias(name)
}

/// `true` when `name` is an international MiniMax alias (`minimax`, `minimax-intl`, ...).
pub fn is_minimax_intl_alias(name: &str) -> bool {
    matches!(
        name,
        "minimax"
            | "minimax-intl"
            | "minimax-io"
            | "minimax-global"
            | "minimax-oauth"
            | "minimax-portal"
            | "minimax-oauth-global"
            | "minimax-portal-global"
    )
}

/// `true` when `name` is a China-region MiniMax alias (`minimax-cn`, ...).
pub fn is_minimax_cn_alias(name: &str) -> bool {
    matches!(
        name,
        "minimax-cn" | "minimaxi" | "minimax-oauth-cn" | "minimax-portal-cn"
    )
}

/// `true` when `name` is any MiniMax alias (international or China-region).
pub fn is_minimax_alias(name: &str) -> bool {
    is_minimax_intl_alias(name) || is_minimax_cn_alias(name)
}

/// `true` when `name` is an international Moonshot / Kimi alias.
pub fn is_moonshot_intl_alias(name: &str) -> bool {
    matches!(
        name,
        "moonshot-intl" | "moonshot-global" | "kimi-intl" | "kimi-global"
    )
}

/// `true` when `name` is a China-region Moonshot / Kimi alias.
pub fn is_moonshot_cn_alias(name: &str) -> bool {
    matches!(name, "moonshot" | "kimi" | "moonshot-cn" | "kimi-cn")
}

/// `true` when `name` is any Moonshot / Kimi alias.
pub fn is_moonshot_alias(name: &str) -> bool {
    is_moonshot_intl_alias(name) || is_moonshot_cn_alias(name)
}

/// `true` when `name` is a China-region Qwen / DashScope alias.
pub fn is_qwen_cn_alias(name: &str) -> bool {
    matches!(name, "qwen" | "dashscope" | "qwen-cn" | "dashscope-cn")
}

/// `true` when `name` is an international Qwen / DashScope alias.
pub fn is_qwen_intl_alias(name: &str) -> bool {
    matches!(
        name,
        "qwen-intl" | "dashscope-intl" | "qwen-international" | "dashscope-international"
    )
}

/// `true` when `name` is a US-region Qwen / DashScope alias.
pub fn is_qwen_us_alias(name: &str) -> bool {
    matches!(name, "qwen-us" | "dashscope-us")
}

/// `true` when `name` is a Qwen OAuth alias (`qwen-code`, `qwen-oauth`, ...).
pub fn is_qwen_oauth_alias(name: &str) -> bool {
    matches!(name, "qwen-code" | "qwen-oauth" | "qwen_oauth")
}

/// `true` when `name` is a Bailian / Aliyun alias.
pub fn is_bailian_alias(name: &str) -> bool {
    matches!(name, "bailian" | "aliyun-bailian" | "aliyun")
}

/// `true` when `name` is any Qwen / DashScope alias.
pub fn is_qwen_alias(name: &str) -> bool {
    is_qwen_cn_alias(name)
        || is_qwen_intl_alias(name)
        || is_qwen_us_alias(name)
        || is_qwen_oauth_alias(name)
}

/// `true` when `name` is a Qianfan / Baidu alias.
pub fn is_qianfan_alias(name: &str) -> bool {
    matches!(name, "qianfan" | "baidu")
}

/// `true` when `name` is a Doubao / Volcengine / Ark alias.
pub fn is_doubao_alias(name: &str) -> bool {
    matches!(name, "doubao" | "volcengine" | "ark" | "doubao-cn")
}

/// Map a China-region provider alias to its canonical provider name
/// (e.g. `zhipu-cn` → `glm`), or `None` when it is not a China alias.
pub fn canonical_china_provider_name(name: &str) -> Option<&'static str> {
    if is_qwen_alias(name) {
        Some("qwen")
    } else if is_glm_alias(name) {
        Some("glm")
    } else if is_moonshot_alias(name) {
        Some("moonshot")
    } else if is_minimax_alias(name) {
        Some("minimax")
    } else if is_zai_alias(name) {
        Some("zai")
    } else if is_qianfan_alias(name) {
        Some("qianfan")
    } else if is_doubao_alias(name) {
        Some("doubao")
    } else if is_bailian_alias(name) {
        Some("bailian")
    } else {
        None
    }
}

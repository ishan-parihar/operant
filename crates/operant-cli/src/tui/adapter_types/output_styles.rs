#[derive(Debug, Clone)]
pub struct StyleInfo {
    pub name: String,
    pub label: String,
    pub description: String,
}

pub fn builtin_styles() -> Vec<StyleInfo> {
    vec![StyleInfo {
        name: "default".to_string(),
        label: "Default".to_string(),
        description: "Standard theme".to_string(),
    }]
}

pub fn find_style<'a>(styles: &'a [StyleInfo], name: &str) -> Option<&'a StyleInfo> {
    styles.iter().find(|s| s.name == name)
}

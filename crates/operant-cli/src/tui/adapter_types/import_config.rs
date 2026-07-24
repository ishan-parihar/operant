
#[derive(Debug, Clone)]
pub struct ImportPaths {}

impl ImportPaths {
    pub fn detect() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportSelection {
    Both,
    Settings,
    ClaudeMd,
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub settings: bool,
    pub claude_md: bool,
    pub auth: bool,
}

pub fn build_import_preview(_sel: ImportSelection) -> Result<ImportPreview, String> {
    Ok(ImportPreview {
        settings: false,
        claude_md: false,
        auth: false,
    })
}

pub fn execute_import(sel: ImportSelection) -> Result<ImportResult, String> {
    let _ = sel;
    Ok(ImportResult {
        imported_fields: vec![],
    })
}

pub fn summarize_import_result(_result: &ImportResult, _paths: &ImportPaths) -> String {
    "Import completed".to_string()
}

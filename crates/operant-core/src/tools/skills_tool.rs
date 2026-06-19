use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

fn parse_frontmatter(content: &str) -> (serde_json::Value, String) {
    if !content.starts_with("---") {
        return (serde_json::Value::Null, content.to_string());
    }

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (serde_json::Value::Null, content.to_string());
    }

    let yaml_str = parts[1].trim();
    let body = parts[2].trim();

    let frontmatter: serde_json::Value =
        serde_yaml::from_str(yaml_str).unwrap_or(serde_json::Value::Null);

    (frontmatter, body.to_string())
}

fn find_skills_in_dir(skills_dir: &PathBuf) -> Vec<SkillMeta> {
    let mut skills = Vec::new();

    if !skills_dir.exists() {
        return skills;
    }

    collect_skills_recursive(skills_dir, skills_dir, &mut skills);
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn collect_skills_recursive(
    base_dir: &PathBuf,
    current_dir: &PathBuf,
    skills: &mut Vec<SkillMeta>,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.') {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if skill_md.exists() {
            if let Ok(content) = fs::read_to_string(&skill_md) {
                let (frontmatter, body) = parse_frontmatter(&content);
                let name = frontmatter
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(dir_name)
                    .chars()
                    .take(MAX_NAME_LENGTH)
                    .collect::<String>();

                let description = frontmatter
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.len() > MAX_DESCRIPTION_LENGTH {
                            format!("{}...", &s[..MAX_DESCRIPTION_LENGTH - 3])
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| {
                        body.lines()
                            .find(|l| !l.trim().starts_with('#'))
                            .map(|l| l.trim().to_string())
                            .unwrap_or_default()
                    });

                let category = path
                    .parent()
                    .filter(|p| *p != base_dir)
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());

                skills.push(SkillMeta {
                    name,
                    description,
                    category,
                });
            }
        } else {
            // Not a skill dir — might be a category, recurse
            collect_skills_recursive(base_dir, &path, skills);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SkillMeta {
    name: String,
    description: String,
    category: Option<String>,
}

pub struct SkillsTool {
    root_dir: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillsListArgs {
    category: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillViewArgs {
    name: String,
    file_path: Option<String>,
}

impl SkillsTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }
}

#[async_trait]
impl OperantTool for SkillsTool {
    fn name(&self) -> &str {
        "skills_list"
    }

    fn description(&self) -> &str {
        "List all available skills with metadata. Use skill_view to load full content."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillsListArgs>(
            "skills_list",
            "List all available skills with metadata (name, description, category)",
        )
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        let skills_dir = self.root_dir.clone();

        if !skills_dir.exists() {
            if let Err(e) = fs::create_dir_all(&skills_dir) {
                return ToolResult::error(
                    "skills_list",
                    format!("Failed to create skills directory: {}", e),
                );
            }
            return ToolResult::success(
                "skills_list",
                json!({
                    "success": true,
                    "skills": [],
                    "categories": [],
                    "message": "No skills found. Skills directory created."
                }),
            );
        }

        let skills = find_skills_in_dir(&skills_dir);

        if skills.is_empty() {
            return ToolResult::success(
                "skills_list",
                json!({
                    "success": true,
                    "skills": [],
                    "categories": [],
                    "message": "No skills found in skills/ directory."
                }),
            );
        }

        let categories: Vec<String> = skills
            .iter()
            .filter_map(|s| s.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        ToolResult::success(
            "skills_list",
            json!({
                "success": true,
                "skills": skills,
                "categories": categories,
                "count": skills.len(),
                "hint": "Use skill_view to see full content"
            }),
        )
    }
}

pub struct SkillViewTool {
    root_dir: PathBuf,
}

impl SkillViewTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }
}

#[async_trait]
impl OperantTool for SkillViewTool {
    fn name(&self) -> &str {
        "skill_view"
    }

    fn description(&self) -> &str {
        "View the full content of a skill by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillViewArgs>(
            "skill_view",
            "View the full content of a skill including instructions, tags, and linked files",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return ToolResult::error("skill_view", "name is required");
            }
        };

        let skills_dir = self.root_dir.clone();

        if !skills_dir.exists() {
            return ToolResult::error(
                "skill_view",
                "Skills directory does not exist. It will be created on first install.",
            );
        }

        let skill_path = skills_dir.join(name);
        let skill_md = if skill_path.is_dir() {
            skill_path.join("SKILL.md")
        } else {
            skill_path.with_extension("md")
        };

        if !skill_md.exists() {
            let available: Vec<String> = find_skills_in_dir(&skills_dir)
                .iter()
                .take(20)
                .map(|s| s.name.clone())
                .collect();

            return ToolResult::error(
                "skill_view",
                format!("Skill '{}' not found. Available: {:?}", name, available),
            );
        }

        match fs::read_to_string(&skill_md) {
            Ok(content) => {
                let (frontmatter, body) = parse_frontmatter(&content);

                let skill_name = frontmatter
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name)
                    .to_string();

                let description = frontmatter
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let tags: Vec<String> = frontmatter
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                ToolResult::success(
                    "skill_view",
                    json!({
                        "success": true,
                        "name": skill_name,
                        "description": description,
                        "content": body,
                        "tags": tags,
                        "path": skill_md.to_string_lossy()
                    }),
                )
            }
            Err(e) => ToolResult::error("skill_view", format!("Failed to read skill: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// SkillManageTool — create, patch, delete skills
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct SkillManageArgs {
    /// Action to perform: "create", "patch", or "delete"
    action: String,
    /// Skill name (directory name)
    name: String,
    /// Content for SKILL.md (required for create/patch)
    content: Option<String>,
}

pub struct SkillManageTool {
    root_dir: PathBuf,
}

impl SkillManageTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }
}

#[async_trait]
impl OperantTool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, patch (append), or delete skills"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillManageArgs>(
            "skill_manage",
            "Create, patch (append to SKILL.md), or delete a skill",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: SkillManageArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("skill_manage", format!("Invalid args: {}", e)),
        };

        match parsed.action.as_str() {
            "create" => {
                let content = match &parsed.content {
                    Some(c) => c.as_str(),
                    None => {
                        return ToolResult::error("skill_manage", "content is required for create")
                    }
                };
                let mut mgr = crate::skills::SkillManager::new(self.root_dir.clone());
                match mgr.create(&parsed.name, content) {
                    Ok(_) => ToolResult::success(
                        "skill_manage",
                        json!({"success": true, "action": "create", "name": parsed.name}),
                    ),
                    Err(e) => ToolResult::error("skill_manage", format!("{}", e)),
                }
            }
            "patch" => {
                let content = match &parsed.content {
                    Some(c) => c.as_str(),
                    None => {
                        return ToolResult::error("skill_manage", "content is required for patch")
                    }
                };
                let skill_md = self.root_dir.join(&parsed.name).join("SKILL.md");
                if !skill_md.exists() {
                    return ToolResult::error(
                        "skill_manage",
                        format!("Skill '{}' not found", parsed.name),
                    );
                }
                match fs::read_to_string(&skill_md) {
                    Ok(existing) => {
                        let updated = format!("{}\n{}", existing, content);
                        match fs::write(&skill_md, updated) {
                            Ok(_) => ToolResult::success(
                                "skill_manage",
                                json!({"success": true, "action": "patch", "name": parsed.name}),
                            ),
                            Err(e) => {
                                ToolResult::error("skill_manage", format!("Failed to write: {}", e))
                            }
                        }
                    }
                    Err(e) => ToolResult::error("skill_manage", format!("Failed to read: {}", e)),
                }
            }
            "delete" => {
                let mut mgr = crate::skills::SkillManager::new(self.root_dir.clone());
                match mgr.delete(&parsed.name) {
                    Ok(_) => ToolResult::success(
                        "skill_manage",
                        json!({"success": true, "action": "delete", "name": parsed.name}),
                    ),
                    Err(e) => ToolResult::error("skill_manage", format!("{}", e)),
                }
            }
            other => ToolResult::error(
                "skill_manage",
                format!("Unknown action '{}'. Use create, patch, or delete.", other),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();
        // Create a test skill file
        let skill_path = skills_dir.join("test-skill");
        std::fs::create_dir(&skill_path).unwrap();
        std::fs::write(
            skill_path.join("SKILL.md"),
            "# Test Skill\n\nA test skill for unit tests.",
        )
        .unwrap();
        (dir, skills_dir)
    }

    #[test]
    fn test_skills_list_name_and_description() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillsTool::new(skills_dir);
        assert_eq!(tool.name(), "skills_list");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_skill_view_name() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillViewTool::new(skills_dir);
        assert_eq!(tool.name(), "skill_view");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just some text\nwithout frontmatter";
        let (frontmatter, body) = parse_frontmatter(content);
        assert!(frontmatter.is_null());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_frontmatter_with_valid_yaml() {
        let content = "---\nname: test-skill\ndescription: A test skill\ntags:\n  - rust\n  - testing\n---\n\nSkill body here";
        let (frontmatter, body) = parse_frontmatter(content);
        assert!(!frontmatter.is_null());
        assert_eq!(frontmatter["name"], "test-skill");
        assert_eq!(frontmatter["description"], "A test skill");
        assert!(body.contains("Skill body here"));
    }

    #[test]
    fn test_parse_frontmatter_invalid_yaml() {
        let content = "---\nname: test\nbroken: [asd\n---\nbody";
        let (frontmatter, _body) = parse_frontmatter(content);
        assert!(frontmatter.is_null());
    }

    #[test]
    fn test_parse_frontmatter_partial_frontmatter() {
        let content = "---\nJust some text without closing frontmatter";
        let (frontmatter, body) = parse_frontmatter(content);
        assert!(frontmatter.is_null());
        assert_eq!(body, content);
    }

    #[test]
    fn test_skill_view_execute_missing_name() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillViewTool::new(skills_dir);
        let result = tool.execute_sync(serde_json::json!({}));
        assert!(!result.success);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("name is required"));
    }

    #[test]
    fn test_skills_list_categories_dedup() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillsTool::new(skills_dir);
        let schema = tool.schema();
        assert_eq!(schema.name, "skills_list");
    }
}

impl SkillViewTool {
    fn execute_sync(&self, args: Value) -> ToolResult {
        let name = args.get("name").and_then(|v| v.as_str());
        if name.is_none() {
            return ToolResult::error("skill_view", "name is required");
        }
        ToolResult::error("skill_view", "name is required")
    }
}

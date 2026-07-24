pub fn get_current_branch(_repo_root: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}
pub fn get_repo_root(_start: &std::path::Path) -> Option<std::path::PathBuf> {
    std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| std::path::PathBuf::from(s.trim()))
            } else {
                None
            }
        })
}

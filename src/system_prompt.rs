use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AnyResult;

pub fn build_system_prompt(cwd: &Path) -> AnyResult<Vec<String>> {
    let mut parts = Vec::new();
    for path in find_agents_md(cwd)? {
        let content = fs::read_to_string(&path)?;
        parts.push(agents_md_template(&path, &content));
    }
    Ok(parts)
}

fn find_agents_md(cwd: &Path) -> AnyResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let path = current.join("AGENTS.md");
        if path.is_file() {
            paths.push(fs::canonicalize(&path)?);
        }
        dir = current.parent();
    }
    Ok(paths)
}

fn agents_md_template(path: &Path, content: &str) -> String {
    format!("Instructions from {}:\n{}", path.display(), content)
}

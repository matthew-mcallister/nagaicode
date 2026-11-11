use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Dirs;
use crate::error::AnyResult;
use crate::session::Session;

pub(crate) struct SystemPromptContext<'a> {
    pub(crate) session: &'a Session,
    pub(crate) cwd: &'a Path,
    pub(crate) dirs: &'a Dirs,
}

pub(crate) fn build_system_prompt(context: SystemPromptContext<'_>) -> AnyResult<Vec<String>> {
    let session_storage = context.dirs.session_storage(context.session.id);
    let mut parts = vec![format!(
        "Session storage path: {}",
        session_storage.display()
    )];
    for path in find_agents_md(context.cwd)? {
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

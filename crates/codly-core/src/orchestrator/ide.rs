//! The workspace IDE facade: every call names an agent, and the root is that agent's
//! worktree (the open repo itself for agents without one, like the Lead). All file work is
//! `crate::ide`, run on a blocking thread; git reads go through `Git`.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use super::{Core, Inner};
use crate::ide::{self, IdeChange, IdeChangeKind, IdeEntry, IdeFile, IdeInfo, IdeSearch};

impl Inner {
    /// The directory the IDE of `agent_id` is confined to.
    fn ide_root(&self, agent_id: &str) -> Result<PathBuf> {
        let agent = self.agent(agent_id)?;
        let root = match agent.worktree {
            Some(wt) => PathBuf::from(wt),
            None => PathBuf::from(self.require_ws()?.repo_path),
        };
        if !root.is_dir() {
            return Err(anyhow!("{}'s workspace is gone ({})", agent.name, root.display()));
        }
        Ok(root)
    }

    /// Like `ide_root`, for calls that run git there: an agent's worktree must still be a
    /// real linked worktree (it can rewrite its own `.git`; see `Git::check_worktree`).
    async fn ide_git_root(&self, agent_id: &str) -> Result<PathBuf> {
        let root = self.ide_root(agent_id)?;
        if self.agent(agent_id)?.worktree.is_some() {
            self.git.check_worktree(Path::new(&self.require_ws()?.repo_path), &root).await?;
        }
        Ok(root)
    }

    async fn ide_changes(&self, root: &Path) -> Result<Vec<IdeChange>> {
        Ok(ide::parse_status(&self.git.status_z(root).await?))
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}

impl Core {
    /// Header facts for the IDE's Home tab.
    pub async fn ide_info(&self, agent_id: &str) -> Result<IdeInfo> {
        let root = self.inner.ide_git_root(agent_id).await?;
        let agent = self.inner.agent(agent_id)?;
        let branch = self.inner.git.current_branch(&root).await.ok().flatten();
        let changed = self.inner.ide_changes(&root).await.map(|c| c.len()).unwrap_or(0);
        Ok(IdeInfo {
            agent_id: agent.id,
            agent_name: agent.name,
            root: root.display().to_string(),
            project: self.inner.workspace().map(|w| w.name).unwrap_or_default(),
            branch,
            changed: changed as i64,
        })
    }

    pub async fn ide_list(&self, agent_id: &str, path: &str) -> Result<Vec<IdeEntry>> {
        let (root, path) = (self.inner.ide_root(agent_id)?, path.to_string());
        blocking(move || ide::list_dir(&root, &path)).await
    }

    pub async fn ide_read(&self, agent_id: &str, path: &str) -> Result<IdeFile> {
        let (root, path) = (self.inner.ide_root(agent_id)?, path.to_string());
        blocking(move || ide::read_file(&root, &path)).await
    }

    pub async fn ide_write(&self, agent_id: &str, path: &str, content: String) -> Result<()> {
        let (root, path) = (self.inner.ide_root(agent_id)?, path.to_string());
        blocking(move || ide::write_file(&root, &path, &content)).await
    }

    pub async fn ide_create(&self, agent_id: &str, path: &str) -> Result<()> {
        let (root, path) = (self.inner.ide_root(agent_id)?, path.to_string());
        blocking(move || ide::create_file(&root, &path)).await
    }

    pub async fn ide_search(&self, agent_id: &str, query: &str) -> Result<IdeSearch> {
        let (root, query) = (self.inner.ide_root(agent_id)?, query.to_string());
        blocking(move || ide::search(&root, &query)).await
    }

    /// Uncommitted changes in the agent's worktree.
    pub async fn ide_changes(&self, agent_id: &str) -> Result<Vec<IdeChange>> {
        let root = self.inner.ide_git_root(agent_id).await?;
        self.inner.ide_changes(&root).await
    }

    /// Unified diff of one changed file against HEAD.
    pub async fn ide_diff(&self, agent_id: &str, path: &str) -> Result<String> {
        let root = self.inner.ide_git_root(agent_id).await?;
        let rel = path.to_string();
        let (r, p) = (root.clone(), rel.clone());
        blocking(move || ide::confine(&r, &p).map(|_| ())).await?;
        let untracked = self.inner.ide_changes(&root).await?.iter().any(|c| c.path == rel && c.kind == IdeChangeKind::Untracked);
        self.inner.git.diff_file(&root, &rel, untracked).await
    }
}

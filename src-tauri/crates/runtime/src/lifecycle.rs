//! Workspace effects composed around the shared board and cleanup rules.
use crate::{dock::DockLaunch, host::Host, terminal::TerminalService};
use prometeu_core::{
    board::{Choice, Workspace},
    workspace_lifecycle::{has_worktree, Change, Cleanable},
};
use std::path::Path;

impl Host {
    pub(crate) fn change_workspace(&mut self, id: &str, change: Change) -> Result<(), String> {
        let catalog = self.catalog.change(id, change)?;
        self.application_events.publish_board(catalog.board)
    }
    fn stop_workspace(&mut self, workspace: &Workspace) -> Result<(), String> {
        for tab in &workspace.tabs {
            self.pending_launches.remove(&tab.id);
        }
        for (key, dock) in &mut self.docks {
            if key
                .strip_prefix(&workspace.id)
                .is_some_and(|suffix| suffix.starts_with(':'))
            {
                stop_terminal(dock)?;
            }
        }
        for tab in &workspace.tabs {
            if let Some(context) = self.contexts.get_mut(&tab.id) {
                stop_terminal(&mut context.terminals)?;
                context.runtime.stop()?;
            }
        }
        Ok(())
    }
    pub(crate) fn archive_workspace(
        &mut self,
        id: &str,
        archived: bool,
        finish: bool,
    ) -> Result<(), String> {
        self.admit_destructive_operation()?;
        let workspace = self.catalog.get(id)?.clone();
        if archived {
            self.stop_workspace(&workspace)?;
            // Cleaned workspaces retain their history but no longer own script resources.
            if !workspace.cleaned {
                let primary = workspace.primary();
                let declarations = self
                    .services
                    .settings
                    .read(Path::new(&primary.worktree), Path::new(&primary.path));
                self.factory.archive(
                    Path::new(&primary.worktree),
                    DockLaunch {
                        command: declarations.archive,
                        environment: prometeu_files::scripts::workspace_env(&workspace),
                        header: None,
                        name: None,
                    },
                )?;
            }
        }
        self.change_workspace(
            id,
            match finish {
                true => Change::Finish,
                false => Change::Archive(archived),
            },
        )
    }
    pub(crate) fn remove_workspace(&mut self, id: &str) -> Result<(), String> {
        self.admit_destructive_operation()?;
        let workspace = self.catalog.get(id)?.clone();
        self.stop_workspace(&workspace)?;
        // Keep retained contexts until host shutdown; the diagnostic selection can still own one.
        // Removing a card does not remove its transcript, Git branch or files.
        self.change_workspace(id, Change::Remove)
    }
    pub(crate) fn retune_tab(&mut self, id: &str, tab: &str, choice: Choice) -> Result<(), String> {
        let mut workspace = self.catalog.get(id)?.clone();
        workspace.retune(tab, choice.clone())?;
        self.admit_application_input(id)?;
        if let Some(context) = self.contexts.get_mut(tab) {
            context.runtime.stop()?;
        }
        let catalog = self.catalog.change(
            id,
            Change::Retune {
                tab: tab.into(),
                choice,
            },
        )?;
        if let Some(context) = self.contexts.get_mut(tab) {
            self.factory.retune(&mut context.runtime, &workspace, tab)?;
        }
        self.application_events.publish_board(catalog.board)
    }
    pub(crate) fn cleanup_list(&self) -> Vec<Cleanable> {
        self.catalog
            .snapshot()
            .board
            .workspaces
            .iter()
            .filter(|w| has_worktree(w))
            .map(|w| self.services.cleanup.inspect(w))
            .collect()
    }
    pub(crate) fn cleanup_workspace(&mut self, id: &str, force: bool) -> Result<(), String> {
        self.admit_destructive_operation()?;
        let workspace = self.catalog.get(id)?.clone();
        if workspace.cleaned {
            return Ok(());
        }
        self.services.cleanup.check(&workspace, force)?;
        // WSL currently creates single-repository worktrees only. A grouping root needs its
        // own ownership validation before the multi-repository launcher can be admitted.
        if workspace.multi() {
            return Err("application_multi_repository_unsupported".into());
        }
        self.stop_workspace(&workspace)?;
        // Even a workspace archived during Setup needs a retained empty transcript store.
        // Bind every tab while its directory still exists, before Git removes the checkout.
        for tab in &workspace.tabs {
            self.session_context(&tab.id)?;
        }
        self.services.cleanup.remove(&workspace)?;
        self.change_workspace(id, Change::Cleaned)
    }
}
fn stop_terminal(terminal: &mut TerminalService) -> Result<(), String> {
    if let Some(id) = terminal.current()?["id"].as_str() {
        terminal.stop(id)?;
    }
    Ok(())
}

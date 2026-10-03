//! Application discovery over registered provider ports and a private account registry.
use prometeu_core::{
    accounts::{key, Account, AccountRegistry, Identity},
    agents::{
        AgentCapabilities, AgentDescriptor, Agents, AuthMethod, CatalogError, ModelCatalog,
        ProviderDiscovery,
    },
    board::ProviderId,
    command::QueryLauncher,
    error::code,
};
use prometeu_protocols::{
    account::parse_account,
    catalog::{query_codex, CatalogProcess},
};
use serde_json::{json, Value};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct Discovery {
    pub accounts: Arc<AccountRegistry>,
    pub providers: Vec<Arc<dyn ProviderDiscovery>>,
}
impl Discovery {
    pub fn provider(&self, id: ProviderId) -> Result<&dyn ProviderDiscovery, String> {
        self.providers
            .iter()
            .find(|p| p.descriptor().id == id)
            .map(Arc::as_ref)
            .ok_or_else(|| code("err.modelsCatalog.unavailable"))
    }
    pub fn agents(&self) -> Agents {
        Agents {
            providers: self.providers.iter().map(|p| p.descriptor()).collect(),
        }
    }
    pub fn models(&self, id: ProviderId) -> Result<ModelCatalog, CatalogError> {
        let provider = self
            .provider(id)
            .map_err(|_| CatalogError::new("unavailable"))?;
        let account = self
            .accounts
            .active(id)
            .map_err(|_| CatalogError::new("noAccount"))?;
        provider.models(&account)
    }
    pub fn validate_model(
        &self,
        launch: &prometeu_core::session::launch::Launch,
    ) -> Result<(), String> {
        let catalog = self.models(launch.agent).map_err(|error| {
            format!(
                "application-error:{}",
                serde_json::to_string(&error).unwrap()
            )
        })?;
        let selected = catalog.models.iter().find(|model| model.id == launch.model);
        if launch.model.is_empty() && launch.effort.is_empty() {
            return Ok(());
        }
        let selected = selected.ok_or_else(|| code("err.modelsCatalog.unavailable"))?;
        if !launch.effort.is_empty() && !selected.efforts.contains(&launch.effort) {
            return Err(code("err.modelsCatalog.unavailable"));
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Value, String> {
        let mut data = self.accounts.snapshot()?;
        data.accounts
            .retain(|account| self.provider(account.provider).is_ok());
        data.active
            .retain(|_, id| data.accounts.iter().any(|account| &account.id == id));
        Ok(json!({"accounts":data.accounts,"active":data.active,"login":null}))
    }
    pub fn refresh(&self) -> Result<Value, String> {
        let snapshot = self.accounts.snapshot()?;
        for account in snapshot.accounts {
            let Ok(provider) = self.provider(account.provider) else {
                continue;
            };
            let identity = provider.identity(&account)?;
            self.accounts.update(|data| {
                if let Some(current) = data
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == account.id && a.revision == account.revision)
                {
                    current.identity = identity;
                }
                Ok(())
            })?;
        }
        self.snapshot()
    }
    pub fn select(&self, id: &str) -> Result<Value, String> {
        let account = self.accounts.find(id)?;
        self.provider(account.provider)?;
        self.accounts.update(|data| data.select(id))?;
        self.snapshot()
    }
    pub fn remove(&self, id: &str) -> Result<Value, String> {
        self.accounts.update(|data| data.remove(id))?;
        self.snapshot()
    }
    pub fn attach(&self, id: ProviderId, method: &str) -> Result<Value, String> {
        let provider = self.provider(id)?;
        if !provider
            .descriptor()
            .auth_methods
            .iter()
            .any(|m| m.id == method && m.kind == "external")
        {
            return Err(code("err.account.external"));
        }
        let mut account = Account {
            id: key(id).into(),
            provider: id,
            revision: 0,
            auth_method: Some(method.into()),
            key_suffix: None,
            identity: Identity::default(),
        };
        account.identity = provider.identity(&account)?;
        self.accounts.update(|data| {
            if !data.accounts.iter().any(|a| a.id == account.id) {
                data.accounts.push(account);
            }
            Ok(())
        })?;
        // Attachment never silently selects an account removed by the person.
        self.snapshot()
    }
}

pub struct CodexDiscovery {
    pub executable: PathBuf,
    pub workdir: PathBuf,
    pub queries: Arc<dyn QueryLauncher<Command>>,
}
impl CodexDiscovery {
    fn command(&self, account: &Account) -> Result<Command, String> {
        if account.id != key(ProviderId::Codex) || account.provider != ProviderId::Codex {
            return Err(code("err.account.external"));
        }
        let mut command = Command::new(&self.executable);
        command.arg("app-server").current_dir(&self.workdir);
        Ok(command)
    }
    fn installed(&self) -> bool {
        let executable = |path: &Path| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        };
        match self.executable.is_absolute() {
            true => executable(&self.executable),
            false => std::env::var_os("PATH")
                .into_iter()
                .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
                .any(|path| executable(&path.join(&self.executable))),
        }
    }
}
impl ProviderDiscovery for CodexDiscovery {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: ProviderId::Codex,
            label: "Codex".into(),
            installed: self.installed(),
            models: vec![],
            auth_methods: vec![AuthMethod {
                id: "external".into(),
                kind: "external".into(),
                label: "Codex (WSL)".into(),
            }],
            unavailable_reason: None,
            account_notice: Some(code("account.windows.external")),
            capabilities: AgentCapabilities {
                initial_plan_mode: false,
                workspace_mcp_selection: true,
                workspace_plugin_selection: true,
                resume: true,
                compact: true,
                context_report: true,
                usage_tokens: true,
                usage_cost: false,
                context_window: true,
                approvals: true,
                user_questions: true,
                attachments: true,
            },
        }
    }
    fn models(&self, account: &Account) -> Result<ModelCatalog, CatalogError> {
        let mut command = self
            .command(account)
            .map_err(|_| CatalogError::new("noAccount"))?;
        Ok(ModelCatalog {
            models: query_codex(
                self.queries.as_ref(),
                &mut command,
                Duration::from_secs(20),
                env!("CARGO_PKG_VERSION"),
            )?,
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        })
    }
    fn identity(&self, account: &Account) -> Result<Identity, String> {
        let mut command = self.command(account)?;
        let map_error = |_| code("err.account.status");
        let mut process =
            CatalogProcess::spawn(self.queries.as_ref(), &mut command, Duration::from_secs(20))
                .map_err(map_error)?;
        process.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"prometeu-accounts","version":env!("CARGO_PKG_VERSION")}}})).map_err(map_error)?;
        process.response(1).map_err(map_error)?;
        process
            .send(json!({"method":"initialized"}))
            .map_err(map_error)?;
        process
            .send(json!({"id":2,"method":"account/read","params":{"refreshToken":true}}))
            .map_err(map_error)?;
        parse_account(&process.response(2).map_err(map_error)?)
    }
}

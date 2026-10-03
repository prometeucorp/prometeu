//! Use official app-server authentication without opening a thread or sending prompts.

use crate::{
    accounts::{AuthProcess, Identity, Profile},
    i18n, oauth, paths,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

pub fn user_home() -> PathBuf {
    let configured = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".codex"));
    if configured.is_absolute() {
        configured
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| paths::home())
            .join(configured)
    }
}

pub use prometeu_profiles::codex::account_env;

struct Server {
    process: AuthProcess,
    next: u64,
    completed: Option<Value>,
}

impl Server {
    fn start(
        profile: &Profile,
        timeout: Duration,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut command = Command::new("codex");
        command.arg("app-server").current_dir(paths::home());
        account_env(&mut command, profile);
        Self::from_command(command, timeout, cancel)
    }

    fn from_command(
        command: Command,
        timeout: Duration,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut server = Self {
            process: AuthProcess::spawn(
                &prometeu_process::auxiliary::UnixAuxiliaryLauncher,
                command,
                timeout,
                cancel,
            )?,
            next: 0,
            completed: None,
        };
        server.call(
            "initialize",
            json!({"clientInfo":{"name":"prometeu-accounts","version":env!("CARGO_PKG_VERSION")}}),
        )?;
        server.process.send(&json!({"method":"initialized"}))?;
        Ok(server)
    }

    fn message(&mut self) -> Result<Value, String> {
        loop {
            let line = self
                .process
                .line()?
                .ok_or_else(|| i18n::t("err.account.status"))?;
            if let Ok(value) = serde_json::from_str(&line) {
                return Ok(value);
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        self.process
            .send(&json!({"id":self.next,"method":method,"params":params}))?;
        loop {
            let value = self.message()?;
            if value["id"].as_u64() == Some(self.next) {
                if value.get("error").is_some() {
                    return Err(i18n::t("err.account.status"));
                }
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| i18n::t("err.account.status"));
            }
            if value["method"] == "account/login/completed" {
                self.completed = Some(value["params"].clone());
            }
        }
    }

    fn identity(&mut self) -> Result<Identity, String> {
        parse_account(&self.call("account/read", json!({"refreshToken":true}))?)
    }
}

use prometeu_protocols::account::parse_account;

pub fn account_probe(profile: &Profile) -> Result<(Identity, Option<Value>), String> {
    let mut server = Server::start(
        profile,
        Duration::from_secs(20),
        Arc::new(AtomicBool::new(false)),
    )?;
    let identity = server.identity()?;
    let usage = if identity.connected {
        server.call("account/rateLimits/read", json!({})).ok()
    } else {
        None
    };
    Ok((identity, usage))
}

pub fn login(profile: &Profile, cancel: Arc<AtomicBool>) -> Result<Identity, String> {
    let mut server = Server::start(profile, Duration::from_secs(600), cancel)?;
    let started = server.call("account/login/start", json!({"type":"chatgpt"}))?;
    let url = started["authUrl"]
        .as_str()
        .ok_or_else(|| i18n::t("err.account.login"))?;
    let parsed = reqwest::Url::parse(url).map_err(|_| i18n::t("err.account.login"))?;
    if parsed.scheme() != "https"
        || !matches!(parsed.host_str(), Some("auth.openai.com" | "chatgpt.com"))
    {
        return Err(i18n::t("err.account.login"));
    }
    oauth::browse(url)?;
    loop {
        let completed = match server.completed.take() {
            Some(value) => value,
            None => {
                let message = server.message()?;
                if message["method"] != "account/login/completed" {
                    continue;
                }
                message["params"].clone()
            }
        };
        if completed["loginId"] != started["loginId"] {
            continue;
        }
        if completed["success"] != true {
            return Err(i18n::t("err.account.login"));
        }
        let identity = server.identity()?;
        if !identity.connected {
            return Err(i18n::t("err.account.login"));
        }
        return Ok(identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires Codex installed; does not log in or send prompts"]
    fn empty_profiles_do_not_inherit_terminal_login() {
        let id = uuid::Uuid::new_v4().to_string();
        let home = std::env::temp_dir().join(format!("prometeu-codex-auth-{id}"));
        paths::ensure_private_dir(&home).unwrap();
        let profile = Profile {
            id,
            provider: crate::state::ProviderId::Codex,
            home: home.clone(),
            managed: true,
            revision: 0,
        };
        let result = account_probe(&profile);
        std::fs::remove_dir_all(home).unwrap();
        assert!(!result.unwrap().0.connected);
    }

    #[test]
    fn account_protocol_153_correlates_responses_without_opening_a_thread() {
        let mut command = Command::new("python3");
        command.args(["-u", "-c", r#"
import sys,json
def read(): return json.loads(sys.stdin.readline())
first=read()
assert first['method']=='initialize'
print(json.dumps({'id':first['id'],'result':{'userAgent':'fixture'}}),flush=True)
assert read()['method']=='initialized'
account=read()
assert account['method']=='account/read'
assert account['params']['refreshToken'] is True
print(json.dumps({'method':'account/updated','params':{'authMode':'chatgpt','planType':'pro'}}),flush=True)
print(json.dumps({'id':account['id'],'result':{'account':{'type':'chatgpt','email':'person@example.com','planType':'pro'},'requiresOpenaiAuth':True}}),flush=True)
read()
"#]);
        let mut server = Server::from_command(
            command,
            Duration::from_secs(5),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let identity = server.identity().unwrap();
        assert!(identity.connected);
        assert_eq!(identity.email.as_deref(), Some("person@example.com"));
        assert!(
            !parse_account(&json!({"account":null,"requiresOpenaiAuth":true}))
                .unwrap()
                .connected
        );
        assert!(parse_account(&json!({})).is_err());
    }
}

//! Application command transport. Command schemas remain owned by the application contract.
use crate::{paths::ApplicationPaths, Target};
use prometeu_core::files::FileLocation;
use serde_json::Value;
use std::sync::Arc;

pub trait ApplicationClient: Send {
    fn initialization_supported(&self) -> bool {
        false
    }
    fn operations_supported(&self) -> bool {
        false
    }
    fn application(&mut self, command: String, args: Value) -> Result<Value, String>;
}

#[derive(Debug, PartialEq)]
pub enum ApplicationResponse {
    Json(Value),
    Bytes(Vec<u8>),
}

pub trait FileManager: Send + Sync {
    fn reveal(&self, path: &str, dir: bool) -> Result<(), String>;
}
pub trait AttachmentClipboard: Send + Sync {
    fn files(&self) -> Result<Vec<String>, String>;
}

/// Native effects at the application boundary; all other commands retain their WSL contract.
pub struct NativeApplication {
    pub paths: Arc<dyn ApplicationPaths>,
    pub files: Arc<dyn FileManager>,
    pub clipboard: Arc<dyn AttachmentClipboard>,
    pub consent: Arc<dyn prometeu_oauth::Consent>,
}
impl NativeApplication {
    pub fn paths(
        &self,
        target: &Target,
        paths: &[String],
        direction: &str,
    ) -> Result<Vec<String>, String> {
        let translate = match direction {
            "linux" => ApplicationPaths::linux,
            "windows" => ApplicationPaths::windows,
            _ => return Err("invalid path direction".into()),
        };
        paths
            .iter()
            .map(|path| translate(self.paths.as_ref(), target, path))
            .collect()
    }
    pub fn request(
        &self,
        target: &Target,
        client: &mut dyn ApplicationClient,
        command: String,
        mut args: Value,
    ) -> Result<ApplicationResponse, String> {
        let mut deferred = crate::operations::Deferred(client);
        let client: &mut dyn ApplicationClient = &mut deferred;
        let result = match command.as_str() {
            "mcp_login" | "mcp_check" | "mcp_logins" | "mcp_logout" => {
                crate::mcp::request(client, self.consent.as_ref(), &command, args)
            }
            "paste_files" => self
                .paths(target, &self.clipboard.files()?, "linux")
                .map(|paths| serde_json::json!(paths)),
            "read_bytes" => {
                return crate::files::read(client, args).map(ApplicationResponse::Bytes)
            }
            "add_project" => {
                let path = args["path"].as_str().ok_or("project path is required")?;
                args["path"] = Value::String(self.paths.linux(target, path)?);
                client.application(command, args)
            }
            "plugin_look" | "plugin_save" => {
                let source = match command.as_str() {
                    "plugin_save" => args
                        .get_mut("plugin")
                        .and_then(|plugin| plugin.get_mut("source")),
                    _ => args.get_mut("source"),
                }
                .ok_or("plugin source is required")?;
                let path = source.as_str().ok_or("plugin source is required")?.trim();
                // Only native Windows paths cross this boundary. URLs and Linux paths retain
                // the original source-validation and tilde semantics in the execution host.
                let bytes = path.as_bytes();
                if path.starts_with("\\\\")
                    || path.starts_with("//")
                    || bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
                {
                    *source = Value::String(self.paths.linux(target, path)?);
                }
                client.application(command, args)
            }
            "reveal_path" => {
                let location: FileLocation =
                    serde_json::from_value(client.application(command, args)?)
                        .map_err(|error| error.to_string())?;
                let path = self.paths.windows(target, &location.path)?;
                self.files.reveal(&path, location.dir).map_err(|_| {
                    prometeu_core::error::with_args(
                        "err.session.openFailed",
                        &[("path", location.path)],
                    )
                })?;
                Ok(Value::Null)
            }
            _ => client.application(command, args),
        };
        result.map(ApplicationResponse::Json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    struct Paths;
    impl prometeu_oauth::Consent for Paths {
        fn authorize(&self, _: &prometeu_oauth::ConsentRequest) -> Result<String, String> {
            Err("No browser in path test".into())
        }
    }
    impl AttachmentClipboard for Paths {
        fn files(&self) -> Result<Vec<String>, String> {
            Ok(vec![r"D:\project".into()])
        }
    }
    impl ApplicationPaths for Paths {
        fn linux(&self, _: &Target, path: &str) -> Result<String, String> {
            assert_eq!(path, r"D:\project");
            Ok("/drive/project".into())
        }
        fn windows(&self, _: &Target, path: &str) -> Result<String, String> {
            match path {
                "/project" => Ok(r"D:\project".into()),
                "/project/file.txt" => Ok(r"D:\project\file.txt".into()),
                _ => Err("path conversion failed".into()),
            }
        }
    }
    #[derive(Default)]
    struct Manager(Mutex<Vec<(String, bool)>>);
    impl FileManager for Manager {
        fn reveal(&self, path: &str, dir: bool) -> Result<(), String> {
            self.0.lock().unwrap().push((path.into(), dir));
            Ok(())
        }
    }
    struct Client {
        reply: Result<Value, String>,
        requests: Vec<(String, Value)>,
    }
    impl ApplicationClient for Client {
        fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
            self.requests.push((command, args));
            self.reply.clone()
        }
    }
    #[test]
    fn plugin_sources_translate_native_paths_and_preserve_execution_sources() {
        let target = Target {
            distribution: "Test".into(),
            root: "/root".into(),
            workdir: "/work".into(),
            executable: "/runtime".into(),
            codex: "/codex".into(),
        };
        let app = NativeApplication {
            paths: Arc::new(Paths),
            files: Arc::new(Manager::default()),
            clipboard: Arc::new(Paths),
            consent: Arc::new(Paths),
        };
        let mut client = Client {
            reply: Ok(Value::Null),
            requests: vec![],
        };
        for args in [Value::Null, json!("invalid"), json!({"plugin":"invalid"})] {
            assert!(app
                .request(&target, &mut client, "plugin_save".into(), args)
                .is_err());
        }
        assert!(client.requests.is_empty());
        for (source, expected) in [
            (r"D:\project", "/drive/project"),
            ("/home/user/plugin", "/home/user/plugin"),
            ("~/plugin", "~/plugin"),
            (
                "https://example.test/plugin.zip",
                "https://example.test/plugin.zip",
            ),
        ] {
            app.request(
                &target,
                &mut client,
                "plugin_look".into(),
                json!({"source":source}),
            )
            .unwrap();
            assert_eq!(
                client.requests.last().unwrap().1,
                json!({"source":expected})
            );
            app.request(
                &target,
                &mut client,
                "plugin_save".into(),
                json!({"plugin":{"id":"example","source":source},"revision":null}),
            )
            .unwrap();
            assert_eq!(
                client.requests.last().unwrap().1,
                json!({"plugin":{"id":"example","source":expected},"revision":null})
            );
        }
    }
    #[test]
    fn native_reveal_uses_only_resolved_entries_and_preserves_the_public_void_contract() {
        let target = Target {
            distribution: "Test".into(),
            root: "/root".into(),
            workdir: "/project".into(),
            executable: "/runtime".into(),
            codex: "/codex".into(),
        };
        let files = Arc::new(Manager::default());
        let app = NativeApplication {
            clipboard: Arc::new(Paths),
            consent: Arc::new(Paths),
            paths: Arc::new(Paths),
            files: files.clone(),
        };
        let mut client = Client {
            reply: Ok(Value::Null),
            requests: Vec::new(),
        };
        assert_eq!(
            app.request(&target, &mut client, "paste_files".into(), Value::Null)
                .unwrap(),
            ApplicationResponse::Json(json!(["/drive/project"]))
        );
        assert!(
            client.requests.is_empty(),
            "clipboard remains a native host effect"
        );
        assert_eq!(
            app.paths(&target, &[r"D:\project".into()], "linux")
                .unwrap(),
            ["/drive/project"]
        );
        assert!(app.paths(&target, &[], "invalid").is_err());
        for (rel, path, dir) in [
            ("", "/project", true),
            ("file.txt", "/project/file.txt", false),
        ] {
            client.reply = Ok(json!({"path":path,"dir":dir}));
            let args = json!({"id":"workspace","rel":rel});
            assert_eq!(
                app.request(&target, &mut client, "reveal_path".into(), args.clone())
                    .unwrap(),
                ApplicationResponse::Json(Value::Null)
            );
            assert_eq!(
                client.requests.last().unwrap(),
                &("reveal_path".into(), args)
            );
        }
        assert_eq!(
            *files.0.lock().unwrap(),
            [
                (r"D:\project".into(), true),
                (r"D:\project\file.txt".into(), false)
            ]
        );
        for reply in [
            Err("outside workspace".into()),
            Ok(Value::Null),
            Ok(json!({"path":"/outside","dir":false})),
        ] {
            client.reply = reply;
            assert!(app
                .request(
                    &target,
                    &mut client,
                    "reveal_path".into(),
                    json!({"id":"workspace","rel":"file.txt"})
                )
                .is_err());
            assert_eq!(files.0.lock().unwrap().len(), 2);
        }
        client.reply = Ok(json!({"id":"registered"}));
        assert_eq!(
            app.request(
                &target,
                &mut client,
                "add_project".into(),
                json!({"path":r"D:\project"})
            )
            .unwrap(),
            ApplicationResponse::Json(json!({"id":"registered"}))
        );
        assert_eq!(
            client.requests.last().unwrap().1,
            json!({"path":"/drive/project"})
        );
        client.reply = Ok(json!({"board":true}));
        assert_eq!(
            app.request(&target, &mut client, "load_board".into(), Value::Null)
                .unwrap(),
            ApplicationResponse::Json(json!({"board":true}))
        );
    }
}

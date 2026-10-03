//! Deferred MCP work keeps HTTP and human consent outside the resident request loop.
use prometeu_tools::{mcp::Server, mcp_auth::Authorization, mcp_probe::Inspection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    Check { server: Server },
    Begin { server: Server },
    Finish { state: String, code: String },
    Logins,
    Logout { id: String },
}
#[derive(Default)]
pub struct Jobs(crate::jobs::Jobs<Value>);
impl Jobs {
    pub fn settle(&self) {
        for (id, result) in self.0.take_ready() {
            self.0.complete(&id, result);
        }
    }
    pub fn idle(&self) -> bool {
        self.settle();
        self.0.idle()
    }
    pub fn running(&self) -> bool {
        self.settle();
        self.0.running()
    }
    pub fn start(
        &self,
        operation: Operation,
        auth: Arc<dyn Authorization>,
        language: String,
    ) -> Result<Value, String> {
        self.settle();
        self.0.start(move || {
            let inspection = Inspection {
                auth: auth.clone(),
                query: Arc::new(prometeu_process::query::UnixQueryLauncher),
            };
            match operation {
                Operation::Check { server } => {
                    serde_json::to_value(inspection.check(&server)).map_err(|e| e.to_string())
                }
                Operation::Begin { server } => inspection.begin(&server).and_then(|mut request| {
                    request.language = language;
                    serde_json::to_value(request).map_err(|e| e.to_string())
                }),
                Operation::Finish { state, code } => {
                    auth.finish(&state, &code).map(|()| Value::Null)
                }
                Operation::Logins => Ok(json!(auth.logged_in())),
                Operation::Logout { id } => auth.forget(&id).map(|()| Value::Null),
            }
        })
    }
    pub fn poll(&self, id: &str) -> Result<Value, String> {
        self.settle();
        self.0.poll(id)
    }
}

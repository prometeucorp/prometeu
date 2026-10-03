//! Desktop composition of shared MCP authorization and native browser consent.
use crate::{i18n, oauth};
use prometeu_tools::mcp_auth::{Authorization, McpAuthorization, PrivateAuthStorage};
use std::{
    net::TcpListener,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
pub(crate) fn service() -> Arc<McpAuthorization> {
    static SERVICE: OnceLock<Arc<McpAuthorization>> = OnceLock::new();
    SERVICE
        .get_or_init(|| {
            Arc::new(McpAuthorization::new(Arc::new(PrivateAuthStorage {
                path: crate::paths::root().join("mcp-auth.json"),
                files: Arc::new(crate::plugins::PrivatePackageFiles),
            })))
        })
        .clone()
}
pub fn logged_in() -> Vec<String> {
    service().logged_in()
}
pub fn forget(id: &str) -> Result<(), String> {
    service().forget(id)
}
pub fn bearer(id: &str) -> Option<String> {
    service().bearer(id)
}
pub fn consent(request: prometeu_oauth::ConsentRequest) -> Result<(), String> {
    let result = (|| {
        let listener =
            TcpListener::bind(("127.0.0.1", oauth::MCP_PORT)).map_err(|e| port(e.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| port(e.to_string()))?;
        oauth::browse(&request.authorize).map_err(|_| i18n::t("err.mcp.auth.noBrowser"))?;
        let code = oauth::wait_for_code(
            &listener,
            oauth::MCP_ROUTE,
            &request.state,
            Instant::now() + Duration::from_secs(300),
            &page,
        )
        .map_err(|e| match e {
            oauth::Denied::Refused => i18n::t("err.mcp.auth.denied"),
            oauth::Denied::Error(why) => i18n::ta("err.mcp.auth.refused", &[("why", why)]),
            oauth::Denied::NoCode => i18n::t("err.mcp.auth.noCode"),
            oauth::Denied::Timeout => i18n::t("err.mcp.auth.timeout"),
            oauth::Denied::Broken(cause) => port(cause),
        })?;
        service().finish(&request.state, &code)
    })();
    service().cancel(&request.state);
    result
}
fn port(cause: String) -> String {
    i18n::ta(
        "err.mcp.auth.port",
        &[("port", oauth::MCP_PORT.to_string()), ("cause", cause)],
    )
}
fn page(ok: bool, why: &str) -> String {
    prometeu_oauth::mcp_page(&i18n::lang(), ok, why)
}

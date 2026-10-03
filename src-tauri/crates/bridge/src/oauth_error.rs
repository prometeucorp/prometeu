//! Preserve the original localized application errors at native consent boundaries.
use prometeu_core::error::{code, with_args};
pub fn browser() -> String {
    code("err.mcp.auth.noBrowser")
}
pub fn port(cause: String) -> String {
    with_args(
        "err.mcp.auth.port",
        &[
            ("port", prometeu_oauth::MCP_PORT.to_string()),
            ("cause", cause),
        ],
    )
}
pub fn denied(error: prometeu_oauth::Denied) -> String {
    match error {
        prometeu_oauth::Denied::Refused => code("err.mcp.auth.denied"),
        prometeu_oauth::Denied::Error(why) => with_args("err.mcp.auth.refused", &[("why", why)]),
        prometeu_oauth::Denied::NoCode => code("err.mcp.auth.noCode"),
        prometeu_oauth::Denied::Timeout => code("err.mcp.auth.timeout"),
        prometeu_oauth::Denied::Broken(cause) => port(cause),
    }
}

//! Adapt the unchanged MCP application commands to bounded resident operations and native consent.
use crate::application::ApplicationClient;
use prometeu_oauth::Consent;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn run(client: &mut dyn ApplicationClient, operation: Value) -> Result<Value, String> {
    let started =
        client.application("mcp_operation_start".into(), json!({"operation":operation}))?;
    let job = started["job"].as_str().ok_or("Invalid MCP operation")?;
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        let result = client.application("mcp_operation_poll".into(), json!({"job":job}))?;
        if result["done"] == true {
            return Ok(result["result"].clone());
        }
        if Instant::now() >= deadline {
            return Err("MCP operation timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn request(
    client: &mut dyn ApplicationClient,
    consent: &dyn Consent,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    let mut operation = args.as_object().cloned().unwrap_or_default();
    let kind = match command {
        "mcp_check" => "check",
        "mcp_logins" => "logins",
        "mcp_logout" => "logout",
        "mcp_login" => "begin",
        _ => return Err("Unknown MCP command".into()),
    };
    operation.insert("kind".into(), json!(kind));
    let result = run(client, json!(operation))?;
    if command != "mcp_login" {
        return Ok(result);
    }
    let request: prometeu_oauth::ConsentRequest =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    let result = consent.authorize(&request).and_then(|code| {
        run(
            client,
            json!({"kind":"finish","state":request.state,"code":code}),
        )
    });
    // Cleanup is idempotent. An uncertain token exchange is never replayed.
    let _ = client.application("mcp_auth_cancel".into(), json!({"state":request.state}));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Client {
        calls: Vec<String>,
        fail_finish: bool,
    }
    impl ApplicationClient for Client {
        fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
            self.calls.push(format!(
                "{command}:{}",
                args["operation"]["kind"].as_str().unwrap_or_default()
            ));
            match command.as_str() {
                "mcp_operation_start" => Ok(json!({"job":args["operation"]["kind"]})),
                "mcp_operation_poll" if args["job"] == "begin" => Ok(
                    json!({"done":true,"result":{"authorize":"https://example.invalid/authorize","state":"state","language":"en"}}),
                ),
                "mcp_operation_poll" if self.fail_finish => Err("transport outcome unknown".into()),
                _ => Ok(Value::Null),
            }
        }
    }
    struct Browser {
        refuse: bool,
    }
    impl Consent for Browser {
        fn authorize(&self, _: &prometeu_oauth::ConsentRequest) -> Result<String, String> {
            match self.refuse {
                true => Err("declined".into()),
                false => Ok("code".into()),
            }
        }
    }
    #[test]
    fn browser_refusal_discards_the_pending_verifier_without_exchange() {
        let mut client = Client {
            calls: vec![],
            fail_finish: false,
        };
        assert_eq!(
            request(
                &mut client,
                &Browser { refuse: true },
                "mcp_login",
                json!({"server":{}})
            ),
            Err("declined".into())
        );
        assert_eq!(
            client.calls,
            vec![
                "mcp_operation_start:begin",
                "mcp_operation_poll:",
                "mcp_auth_cancel:"
            ]
        );
    }
    #[test]
    fn uncertain_exchange_is_not_replayed() {
        let mut client = Client {
            calls: vec![],
            fail_finish: true,
        };
        assert_eq!(
            request(
                &mut client,
                &Browser { refuse: false },
                "mcp_login",
                json!({"server":{}})
            ),
            Err("transport outcome unknown".into())
        );
        assert_eq!(
            client
                .calls
                .iter()
                .filter(|c| c.as_str() == "mcp_operation_start:finish")
                .count(),
            1
        );
        assert_eq!(client.calls.last().unwrap(), "mcp_auth_cancel:");
    }
}

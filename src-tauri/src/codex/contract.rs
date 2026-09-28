//! Synthetic translation scenarios, independent of installed accounts and provider versions.

use super::*;

pub(crate) fn start(resume: Option<String>) -> Start {
    Start {
        cwd: "/contract/worktree".into(),
        resume,
        model: "".into(),
        effort: "".into(),
        plugin_ids: vec![],
        plugin_hook_ids: vec![],
        permission: None,
        access: Default::default(),
        instructions: String::new(),
    }
}

pub(crate) fn events() -> Vec<Value> {
    let mut link = Link::new(Box::new(std::io::sink()), start(None));
    [
        json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"turn"}}}),
        json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"type":"agentMessage","id":"answer","text":""}}}),
        json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"answer","delta":"Reviewed"}}),
        json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"type":"agentMessage","id":"answer","text":"Reviewed"}}}),
        json!({"id":"approval","method":"item/commandExecution/requestApproval","params":{"threadId":"thread","turnId":"turn","itemId":"command","command":"git status"}}),
        json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed"}}}),
    ].iter().flat_map(|frame| link.on_line(&frame.to_string())).collect()
}

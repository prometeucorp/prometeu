//! Backend errors and activity notes use stable codes plus named arguments; the frontend formats
//! them in the selected language. Preserve the String IPC contract with i18n:{code,args}.
//! Non-prefixed external errors remain readable as supplied. The canonical message catalog lives in
//! src/i18n.pt.ts.

use std::sync::Mutex;

/// The frontend sets the global display language at startup and on changes. Only terminal notices,
/// agent-injected warnings, and OAuth pages require complete backend-rendered messages; ordinary UI
/// text uses codes.
static LANG: Mutex<String> = Mutex::new(String::new());

#[tauri::command]
pub fn set_lang(lang: String) {
    *LANG.lock().unwrap_or_else(|e| e.into_inner()) = lang;
}

pub fn lang() -> String {
    LANG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Default to Portuguese until the frontend supplies its language; English is the other supported
/// locale.
pub fn pt() -> bool {
    !lang().starts_with("en")
}

/// Tests modifying the global language must hold this lock to avoid racing concurrent tests.
#[cfg(test)]
pub static TEST_LANG: Mutex<()> = Mutex::new(());

/// Choose localized text only for the few messages rendered entirely by the backend. Other messages
/// use frontend catalog codes.
pub fn pick(pt_br: &str, en: &str) -> String {
    if pt() {
        pt_br.to_string()
    } else {
        en.to_string()
    }
}

/// Encode a message code without arguments.
pub use prometeu_core::error::code as t;

/// Encode a message code with arguments named after the catalog placeholders.
pub use prometeu_core::error::with_args as ta;

/// Wrap external library errors through the catalog without translating their diagnostic text.
pub fn io(cause: impl ToString) -> String {
    ta("err.io", &[("cause", cause.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_language_defaults_to_portuguese() {
        let _guard = TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        set_lang(String::new());
        assert!(pt());
        set_lang("en".into());
        assert!(!pt());
        set_lang("pt-BR".into());
        assert!(pt());
    }

    #[test]
    fn codes_without_arguments_remain_unchanged() {
        assert_eq!(t("err.pty.gone"), r#"i18n:{"code":"err.pty.gone"}"#);
    }

    #[test]
    fn arguments_are_ordered_and_escaped() {
        let got = ta("err.session.notGit", &[("path", "/tmp/a\"b".into())]);
        assert_eq!(
            got,
            r#"i18n:{"args":{"path":"/tmp/a\"b"},"code":"err.session.notGit"}"#
        );
    }
}

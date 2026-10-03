//! Desktop browser and localized page composition over shared OAuth mechanics.
use crate::i18n;
pub use prometeu_oauth::*;

pub fn browse(url: &str) -> Result<(), String> {
    let ok = crate::platform::opener()
        .arg(url)
        .status()
        .map_err(i18n::io)?
        .success();
    ok.then_some(()).ok_or_else(|| String::from("open"))
}

pub fn page(title: &str, text: &str) -> String {
    prometeu_oauth::page(&i18n::lang(), title, text)
}

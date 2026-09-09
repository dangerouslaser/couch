//! User-configured Android app links, never an inferred installed-app catalog.
use crate::{Config, Problem, Provider};
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppShortcut {
    pub name: String,
    pub url: String,
}
/// Accept bounded absolute app links without embedded authority credentials.
/// Custom application schemes are useful on Android; script/file URLs are not.
pub fn valid_app_url(url: &str) -> bool {
    if url.len() > 2048
        || url
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return false;
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if scheme.is_empty()
        || scheme.len() > 32
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
        || rest.is_empty()
        || ["javascript", "data", "file", "content"]
            .iter()
            .any(|s| scheme.eq_ignore_ascii_case(s))
    {
        return false;
    }
    if let Some(authority) = rest.strip_prefix("//") {
        let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') {
            return false;
        }
    } else if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    true
}
impl Config {
    pub(crate) fn validate_app_shortcuts(&self, problems: &mut Vec<Problem>) {
        for (id, apps) in &self.app_shortcuts {
            let at = alloc::format!("app_shortcuts.{id}");
            if !self
                .connection(id)
                .is_some_and(|c| c.provider == Provider::AndroidTv)
            {
                problems.push(Problem {
                    at: at.clone(),
                    message: "App shortcuts require an Android TV connection".into(),
                });
            }
            if apps.len() > 24 {
                problems.push(Problem {
                    at: at.clone(),
                    message: "Use at most 24 app shortcuts per TV".into(),
                });
            }
            for (index, app) in apps.iter().enumerate() {
                if app.name.trim().is_empty()
                    || app.name.chars().count() > 64
                    || app.name.chars().any(char::is_control)
                    || !valid_app_url(&app.url)
                {
                    problems.push(Problem{at:alloc::format!("{at}[{index}]"),message:"Use a name up to 64 characters and an absolute app link without login credentials".into()});
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_are_bounded_absolute_and_without_login_credentials() {
        for value in [
            "https://www.youtube.com/",
            "netflix://browse",
            "vnd.youtube:abc",
        ] {
            assert!(valid_app_url(value));
        }
        for value in [
            "",
            "com.netflix.app",
            "https:",
            "https:foo",
            "https://",
            "https://user:secret@host/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://host/ bad",
        ] {
            assert!(!valid_app_url(value));
        }
        assert!(!valid_app_url(&alloc::format!(
            "https://x/{}",
            "a".repeat(2048)
        )));
        let action = crate::commands::Function::parse("app:https://www.youtube.com/").unwrap();
        assert!(action.supports(&crate::Integration::AndroidTv));
        assert!(!action.supports(&crate::Integration::WebOs));
        assert_eq!(action.id(), "app:https://www.youtube.com/");
    }
    #[test]
    fn defaults_stay_blank_and_shortcuts_need_the_correct_provider() {
        let mut c = Config::default();
        assert!(!serde_json::to_string(&c).unwrap().contains("app_shortcuts"));
        c.app_shortcuts.insert(
            "tv".into(),
            alloc::vec![AppShortcut {
                name: "YouTube".into(),
                url: "https://www.youtube.com/".into()
            }],
        );
        assert!(c.validate().is_err());
        c.connections.push(crate::Connection {
            id: "tv".into(),
            name: "TV".into(),
            provider: Provider::AndroidTv,
        });
        assert!(c.validate().is_ok());
        let restored: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(restored, c);
        c.connections[0].provider = Provider::AppleTv;
        assert!(c.validate().is_err());
    }
}

//! Help > About and the project's pages the Help menu opens. Every address derives from the
//! workspace manifest's `repository`: the UI names a page, never an address, so it cannot open
//! an arbitrary one.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// The project's repository (`repository` in the workspace's `Cargo.toml`).
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// What Help > About shows.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppInfo {
    /// The application's version, as built (the workspace's `version`).
    version: String,
    /// The SPDX expression of the license (`license` in the workspace's `Cargo.toml`).
    license: &'static str,
    /// The project's page.
    repository: &'static str,
}

#[tauri::command]
pub(crate) fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        license: env!("CARGO_PKG_LICENSE"),
        repository: REPOSITORY,
    }
}

/// A page of the project the UI may open in the browser.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProjectPage {
    /// The repository's home (its README).
    Home,
    /// A new issue (the repository has no issue templates: the plain form).
    NewIssue,
    /// How to contribute (`CONTRIBUTING.md`).
    Contributing,
}

impl ProjectPage {
    fn url(self) -> String {
        match self {
            Self::Home => REPOSITORY.to_owned(),
            Self::NewIssue => format!("{REPOSITORY}/issues/new"),
            Self::Contributing => format!("{REPOSITORY}/blob/main/CONTRIBUTING.md"),
        }
    }
}

/// Opens `page` in the system's browser.
#[tauri::command]
pub(crate) async fn open_project_page(app: AppHandle, page: ProjectPage) -> Result<(), String> {
    app.opener()
        .open_url(page.url(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_on_the_repository() {
        assert!(REPOSITORY.starts_with("https://github.com/"));
        assert_eq!(ProjectPage::Home.url(), REPOSITORY);
        assert_eq!(
            ProjectPage::NewIssue.url(),
            format!("{REPOSITORY}/issues/new")
        );
        assert_eq!(
            ProjectPage::Contributing.url(),
            format!("{REPOSITORY}/blob/main/CONTRIBUTING.md")
        );
    }

    #[test]
    fn contributing_page_exists() {
        // The address points at this file of the repository.
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../CONTRIBUTING.md");
        assert!(std::path::Path::new(root).is_file());
    }

    #[test]
    fn pages_are_named_in_camel_case() {
        let page: ProjectPage = serde_json::from_str("\"newIssue\"").unwrap();
        assert_eq!(page, ProjectPage::NewIssue);
        assert!(serde_json::from_str::<ProjectPage>("\"https://example.org\"").is_err());
    }
}

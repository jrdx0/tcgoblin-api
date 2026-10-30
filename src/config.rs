use std::{env, path::PathBuf, sync::Arc};

use crate::{crypto::Cipher, page::PageFetcher};

pub fn env_var(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

pub fn cipher() -> Cipher {
    Cipher::from_base64_key(&env_var("SETTINGS_ENCRYPTION_KEY"))
}

pub fn page_fetcher() -> Arc<PageFetcher> {
    let chrome_executable = env::var("CHROME_EXECUTABLE")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let chrome_download_dir = env::var("CHROME_DOWNLOAD_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ".chromium".to_string());
    let chrome_no_sandbox = env::var("CHROME_NO_SANDBOX").is_ok_and(|v| v == "true");

    let page = Arc::new(PageFetcher::new(
        chrome_executable,
        PathBuf::from(chrome_download_dir),
        chrome_no_sandbox,
    ));

    tokio::spawn({
        let page = page.clone();
        async move {
            if let Err(err) = page.warm_up().await {
                eprintln!("browser warm-up failed: {err}");
            }
        }
    });

    page
}

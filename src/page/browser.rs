use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use chromiumoxide::{
    browser::{Browser, BrowserConfig},
    fetcher::{BrowserFetcher, BrowserFetcherOptions},
};
use futures_util::StreamExt;
use mongodb::bson::oid::ObjectId;
use tokio::sync::OnceCell;

use super::FetchedPage;

const SETTLE_DELAY: Duration = Duration::from_millis(1500);

pub struct BrowserLauncher {
    configured_executable: Option<PathBuf>,
    download_dir: PathBuf,
    no_sandbox: bool,
    executable: OnceCell<PathBuf>,
}

impl BrowserLauncher {
    pub fn new(
        configured_executable: Option<PathBuf>,
        download_dir: PathBuf,
        no_sandbox: bool,
    ) -> Self {
        Self {
            configured_executable,
            download_dir,
            no_sandbox,
            executable: OnceCell::new(),
        }
    }

    pub async fn fetch(&self, url: &str, timeout: Duration) -> Result<FetchedPage, String> {
        let executable = self.executable().await?;
        let profile = ProfileDir::create()?;

        let mut config = BrowserConfig::builder()
            .chrome_executable(executable)
            .user_data_dir(&profile.0)
            .request_timeout(timeout);
        if self.no_sandbox {
            config = config.no_sandbox();
        }
        let config = config
            .build()
            .map_err(|e| format!("invalid browser config: {e}"))?;

        let (mut browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| format!("failed to launch browser: {e}"))?;
        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let result = load(&browser, url).await;

        let _ = browser.close().await;
        let _ = browser.wait().await;
        handler_task.abort();

        result
    }

    pub async fn executable(&self) -> Result<&PathBuf, String> {
        self.executable
            .get_or_try_init(|| async {
                if let Some(path) = &self.configured_executable {
                    return Ok(path.clone());
                }
                download_chromium(&self.download_dir).await
            })
            .await
    }
}

async fn load(browser: &Browser, url: &str) -> Result<FetchedPage, String> {
    let page = browser
        .new_page(url)
        .await
        .map_err(|e| format!("failed to open page: {e}"))?;
    page.wait_for_navigation()
        .await
        .map_err(|e| format!("page did not finish loading: {e}"))?;

    tokio::time::sleep(SETTLE_DELAY).await;

    let html = page
        .content()
        .await
        .map_err(|e| format!("failed to read page HTML: {e}"))?;
    let final_url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| url.to_string());

    Ok(FetchedPage {
        url: final_url,
        html,
    })
}

async fn download_chromium(download_dir: &Path) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(download_dir)
        .await
        .map_err(|e| format!("failed to create {}: {e}", download_dir.display()))?;

    let options = BrowserFetcherOptions::builder()
        .with_path(download_dir)
        .build()
        .map_err(|e| format!("invalid browser fetcher options: {e}"))?;

    let installation = BrowserFetcher::new(options)
        .fetch()
        .await
        .map_err(|e| format!("failed to download Chromium: {e}"))?;

    Ok(installation.executable_path)
}

struct ProfileDir(PathBuf);

impl ProfileDir {
    fn create() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("tcgoblin-chromium-{}", ObjectId::new().to_hex()));
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("failed to create browser profile dir: {e}"))?;
        Ok(Self(path))
    }
}

impl Drop for ProfileDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

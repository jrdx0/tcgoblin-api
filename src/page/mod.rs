mod browser;
mod http;

use std::{path::PathBuf, time::Duration};

use reqwest::Client;
use scraper::{Html, Selector};

use crate::models::product::FetchMode;
use browser::BrowserLauncher;

const MIN_VISIBLE_TEXT: usize = 200;
const NOSCRIPT_TEXT_LIMIT: usize = 1000;
const APP_ROOTS: &str = "#app, #root, #__next, #__nuxt, [data-reactroot]";
const CHALLENGE_TITLES: [&str; 3] = ["just a moment", "attention required", "access denied"];

pub struct FetchedPage {
    pub url: String,
    pub html: String,
}

pub struct Snapshot {
    pub url: String,
    pub html: String,
    pub fetch_mode: FetchMode,
}

pub struct PageFetcher {
    http: Client,
    browser: BrowserLauncher,
}

impl PageFetcher {
    pub fn new(
        chrome_executable: Option<PathBuf>,
        chrome_download_dir: PathBuf,
        chrome_no_sandbox: bool,
    ) -> Self {
        Self {
            http: http::client(),
            browser: BrowserLauncher::new(
                chrome_executable,
                chrome_download_dir,
                chrome_no_sandbox,
            ),
        }
    }

    pub async fn warm_up(&self) -> Result<(), String> {
        self.browser.executable().await.map(|_| ())
    }

    pub async fn fetch_http(&self, url: &str, timeout: Duration) -> Result<FetchedPage, String> {
        tokio::time::timeout(timeout, http::fetch(&self.http, url))
            .await
            .map_err(|_| format!("HTTP request timed out after {}s", timeout.as_secs()))?
    }

    pub async fn fetch_browser(&self, url: &str, timeout: Duration) -> Result<FetchedPage, String> {
        self.browser.executable().await?;

        tokio::time::timeout(timeout, self.browser.fetch(url, timeout))
            .await
            .map_err(|_| format!("browser timed out after {}s", timeout.as_secs()))?
    }

    pub async fn snapshot(
        &self,
        url: &str,
        timeout: Duration,
        forced_mode: Option<FetchMode>,
    ) -> Result<Snapshot, String> {
        match forced_mode {
            Some(FetchMode::Http) => {
                return self.fetch_http(url, timeout).await.map(|page| Snapshot {
                    url: page.url,
                    html: page.html,
                    fetch_mode: FetchMode::Http,
                });
            }
            Some(FetchMode::Browser) => {
                return self.fetch_browser(url, timeout).await.map(|page| Snapshot {
                    url: page.url,
                    html: page.html,
                    fetch_mode: FetchMode::Browser,
                });
            }
            None => {}
        }

        let http_problem = match self.fetch_http(url, timeout).await {
            Ok(page) => match needs_browser(&page.html) {
                None => {
                    return Ok(Snapshot {
                        url: page.url,
                        html: page.html,
                        fetch_mode: FetchMode::Http,
                    });
                }
                Some(reason) => reason.to_string(),
            },
            Err(err) => err,
        };

        self.fetch_browser(url, timeout)
            .await
            .map(|page| Snapshot {
                url: page.url,
                html: page.html,
                fetch_mode: FetchMode::Browser,
            })
            .map_err(|err| format!("{err} (HTTP attempt: {http_problem})"))
    }
}

fn needs_browser(html: &str) -> Option<&'static str> {
    let document = Html::parse_document(html);

    let title = select_text(&document, "title").to_lowercase();
    if CHALLENGE_TITLES.iter().any(|t| title.contains(t)) {
        return Some("page is an antibot challenge");
    }

    let visible = visible_text_len(&document);
    if visible < MIN_VISIBLE_TEXT {
        return Some("page has almost no visible text");
    }

    let roots = Selector::parse(APP_ROOTS).expect("valid selector");
    if document
        .select(&roots)
        .any(|el| el.children().next().is_none() || el.text().all(|t| t.trim().is_empty()))
    {
        return Some("app root element is empty");
    }

    let noscript = select_text(&document, "noscript").to_lowercase();
    if visible < NOSCRIPT_TEXT_LIMIT && noscript.contains("javascript") {
        return Some("page asks to enable JavaScript");
    }

    None
}

fn select_text(document: &Html, selector: &str) -> String {
    let selector = Selector::parse(selector).expect("valid selector");
    document
        .select(&selector)
        .flat_map(|el| el.text())
        .collect()
}

fn visible_text_len(document: &Html) -> usize {
    let body = Selector::parse("body").expect("valid selector");
    let Some(body) = document.select(&body).next() else {
        return 0;
    };

    body.descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let hidden = node.ancestors().any(|a| {
                a.value().as_element().is_some_and(|e| {
                    matches!(e.name(), "script" | "style" | "noscript" | "template")
                })
            });
            (!hidden).then(|| text.split_whitespace().map(str::len).sum::<usize>())
        })
        .sum()
}

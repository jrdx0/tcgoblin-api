use reqwest::{
    Client,
    header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue},
};

use super::FetchedPage;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

pub fn client() -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml;q=0.9,*/*;q=0.8"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("es-MX,es;q=0.9,en;q=0.8"),
    );

    Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .build()
        .expect("failed to build HTTP client")
}

pub async fn fetch(client: &Client, url: &str) -> Result<FetchedPage, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("store responded with HTTP {status}"));
    }

    let url = response.url().to_string();
    let html = response
        .text()
        .await
        .map_err(|e| format!("failed to read HTTP response: {e}"))?;

    Ok(FetchedPage { url, html })
}

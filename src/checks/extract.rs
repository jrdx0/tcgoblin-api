use scraper::{Html, Selector};

pub enum Extraction {
    Found(String),
    NotFound,
    InvalidSelector,
}

pub fn first_text(html: &str, selector: &str) -> Extraction {
    let Ok(selector) = Selector::parse(selector) else {
        return Extraction::InvalidSelector;
    };

    let document = Html::parse_document(html);
    match document.select(&selector).next() {
        Some(element) => Extraction::Found(
            element
                .text()
                .flat_map(str::split_whitespace)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        None => Extraction::NotFound,
    }
}

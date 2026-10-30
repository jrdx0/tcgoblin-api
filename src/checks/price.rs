pub fn parse_price(text: &str) -> Option<f64> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let number: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '.'))
        .collect();

    let number = number.trim_end_matches(['.', ',']).replace(',', "");
    let value: f64 = number.parse().ok()?;

    (value.is_finite() && value > 0.0).then_some(value)
}

use serde_json::{Value, json};
use url::Url;

use crate::{
    models::{
        alert::{Alert, AlertKind},
        product::FailureReason,
    },
    routes::products::to_rfc3339,
};

const COLOR_DROP: u32 = 0x2ECC71;
const COLOR_RISE: u32 = 0xE74C3C;
const COLOR_FAILURE: u32 = 0xE67E22;
const COLOR_RECOVERY: u32 = 0x3498DB;

const MAX_TITLE: usize = 200;
const MAX_FIELD: usize = 300;

pub fn build(alerts: &[Alert]) -> Value {
    json!({ "embeds": alerts.iter().map(embed).collect::<Vec<_>>() })
}

fn embed(alert: &Alert) -> Value {
    let (label, color) = match alert.kind {
        AlertKind::PriceDrop => ("⬇️ Bajó de precio", COLOR_DROP),
        AlertKind::PriceRise => ("⬆️ Subió de precio", COLOR_RISE),
        AlertKind::Failure => ("⚠️ Fallo al revisar", COLOR_FAILURE),
        AlertKind::Recovery => ("✅ Se recuperó", COLOR_RECOVERY),
    };

    let mut description = format!("**{label}**");
    if alert.kind == AlertKind::PriceDrop && alert.below_average {
        description.push_str("\n🏷️ **Oferta**: por debajo del promedio del producto");
    }

    let mut fields = vec![field("Tienda", store_name(&alert.store_url), true)];
    match alert.kind {
        AlertKind::PriceDrop | AlertKind::PriceRise => {
            fields.push(field(
                "Antes",
                money(alert.previous_price, &alert.currency),
                true,
            ));
            fields.push(field("Ahora", money(alert.price, &alert.currency), true));
            if let Some(change) = change_percent(alert.previous_price, alert.price) {
                fields.push(field("Variación", change, true));
            }
        }
        AlertKind::Recovery => {
            fields.push(field(
                "Precio actual",
                money(alert.price, &alert.currency),
                true,
            ));
        }
        AlertKind::Failure => {
            if let Some(failure) = &alert.failure {
                fields.push(field(
                    "Tipo de fallo",
                    failure_label(failure.reason).to_string(),
                    true,
                ));
                if let Some(message) = &failure.message {
                    fields.push(field("Detalle", message.clone(), false));
                }
                let last_success = failure
                    .last_success_at
                    .map(|d| format!("<t:{}:f>", d.timestamp_millis() / 1000))
                    .unwrap_or_else(|| "Nunca".to_string());
                fields.push(field("Última revisión correcta", last_success, false));
            }
        }
    }

    let mut embed = json!({
        "title": truncate(&alert.product_name, MAX_TITLE),
        "description": description,
        "color": color,
        "fields": fields,
        "timestamp": to_rfc3339(alert.created_at),
        "footer": { "text": "TCGoblin" },
    });
    if is_http_url(&alert.store_url) {
        embed["url"] = json!(alert.store_url);
    }
    embed
}

fn field(name: &str, value: String, inline: bool) -> Value {
    let value = if value.trim().is_empty() {
        "—".to_string()
    } else {
        truncate(&value, MAX_FIELD)
    };
    json!({ "name": name, "value": value, "inline": inline })
}

fn failure_label(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::ElementNotFound => "Elemento no encontrado",
        FailureReason::PageUnavailable => "Página no disponible",
        FailureReason::InvalidText => "Texto no válido",
    }
}

fn store_name(store_url: &str) -> String {
    Url::parse(store_url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_else(|| store_url.to_string())
}

fn is_http_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn money(amount: Option<f64>, currency: &str) -> String {
    match amount {
        Some(amount) => format!("${} {currency}", thousands(amount)),
        None => "—".to_string(),
    }
}

fn thousands(amount: f64) -> String {
    let formatted = format!("{:.2}", amount.abs());
    let (integer, decimals) = formatted.split_once('.').unwrap_or((&formatted, "00"));

    let mut grouped = String::new();
    for (i, digit) in integer.chars().enumerate() {
        if i > 0 && (integer.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }

    let sign = if amount < 0.0 { "-" } else { "" };
    format!("{sign}{grouped}.{decimals}")
}

fn change_percent(previous: Option<f64>, current: Option<f64>) -> Option<String> {
    let (previous, current) = (previous?, current?);
    if previous == 0.0 {
        return None;
    }
    let change = (current - previous) / previous * 100.0;
    Some(format!("{change:+.2}%"))
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max - 1).collect();
    truncated.push('…');
    truncated
}

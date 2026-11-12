//! Small helpers to write files exactly like the TS app did.

use serde::Serialize;
use serde_json::Value;

/// Whole-number floats become integers (JS has one number type: `1234`, not `1234.0`).
pub fn js_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(js_numbers).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, js_numbers(v))).collect()),
        x => x,
    }
}

/// `JSON.stringify(x, null, 2)`.
pub fn json_pretty<T: Serialize>(x: &T) -> String {
    serde_json::to_string_pretty(&js_numbers(serde_json::to_value(x).unwrap_or(Value::Null))).unwrap_or_default()
}

/// `JSON.stringify(x)`.
pub fn json_compact<T: Serialize>(x: &T) -> String {
    serde_json::to_string(&js_numbers(serde_json::to_value(x).unwrap_or(Value::Null))).unwrap_or_default()
}

/// `new Date(ms).toISOString().replace(/[:.]/g, '-').slice(0, 19)`, e.g. "2026-11-12T03-10-00".
pub fn iso_id(epoch_ms: f64) -> String {
    let secs = (epoch_ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}", sod / 3600, sod % 3600 / 60, sod % 60)
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn iso_and_numbers() {
        assert_eq!(iso_id(0.0), "1970-01-01T00-00-00");
        assert_eq!(iso_id(1_794_454_200_123.0), "2026-11-12T03-30-00");
        assert_eq!(iso_id(951_782_400_000.0), "2000-02-29T00-00-00");
        assert_eq!(json_compact(&json!({ "a": 1.0, "b": [2.5, 3.0] })), "{\"a\":1,\"b\":[2.5,3]}");
        assert_eq!(json_pretty(&json!({ "a": [] })), "{\n  \"a\": []\n}");
    }
}

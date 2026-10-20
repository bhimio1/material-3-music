//! Small helpers for walking InnerTube's deeply nested JSON without
//! panicking when YouTube moves things around.

use serde_json::Value;

/// Follow a `/`-separated path. Numeric segments index arrays; `-1` is the
/// last element.
pub fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = value;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => {
                if seg == "-1" {
                    a.last()?
                } else {
                    a.get(seg.parse::<usize>().ok()?)?
                }
            }
            _ => return None,
        };
    }
    Some(cur)
}

pub fn str_at<'a>(value: &'a Value, path: &str) -> Option<&'a str> {
    at(value, path)?.as_str()
}

pub fn arr_at<'a>(value: &'a Value, path: &str) -> &'a [Value] {
    at(value, path)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Concatenated text of a `{ "runs": [...] }` or `{ "simpleText": … }`.
pub fn text(value: &Value) -> String {
    if let Some(s) = value.get("simpleText").and_then(Value::as_str) {
        return s.to_string();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
}

pub fn text_at(value: &Value, path: &str) -> String {
    at(value, path).map(text).unwrap_or_default()
}

pub fn runs(value: Option<&Value>) -> &[Value] {
    value
        .and_then(|v| v.get("runs"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Every object stored under `key` anywhere in the tree, in document order.
pub fn find_all<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    collect(value, key, &mut out);
    out
}

fn collect<'a>(value: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    out.push(v);
                } else {
                    collect(v, key, out);
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                collect(v, key, out);
            }
        }
        _ => {}
    }
}

/// Objects stored under any of `keys`, in document order.
pub fn find_any<'a>(value: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    fn go<'a>(value: &'a Value, keys: &[&str], out: &mut Vec<&'a Value>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    if keys.contains(&k.as_str()) {
                        out.push(v);
                    } else {
                        go(v, keys, out);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|v| go(v, keys, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(value, keys, &mut out);
    out
}

/// First object stored under `key` anywhere in the tree.
pub fn find<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if let Some(v) = map.get(key) {
                return Some(v);
            }
            map.values().find_map(|v| find(v, key))
        }
        Value::Array(items) => items.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}

/// Largest thumbnail URL in a `thumbnails` array somewhere below `value`.
pub fn thumbnail(value: &Value) -> Option<String> {
    let thumbs = find(value, "thumbnails")?.as_array()?;
    thumbs
        .iter()
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str)
        .map(normalize_url)
}

fn normalize_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_and_text() {
        let v = json!({"a": {"b": [1, {"runs": [{"text": "Hi "}, {"text": "there"}]}]}});
        assert_eq!(at(&v, "a/b/0"), Some(&json!(1)));
        assert_eq!(text_at(&v, "a/b/-1"), "Hi there");
        assert_eq!(at(&v, "a/x/1"), None);
        assert_eq!(at(&v, "a/b/9"), None);
        assert_eq!(text(&json!({"simpleText": "x"})), "x");
        assert!(arr_at(&v, "nope").is_empty());
    }

    #[test]
    fn deep_find() {
        let v = json!({"x": [{"y": {"target": 1}}, {"target": 2}]});
        assert_eq!(find_all(&v, "target").len(), 2);
        assert_eq!(find(&v, "target"), Some(&json!(1)));
        let t = json!({"thumbnail": {"thumbnails": [
            {"url": "//a/small", "width": 60}, {"url": "https://a/big", "width": 544}]}});
        assert_eq!(thumbnail(&t).as_deref(), Some("https://a/big"));
    }
}

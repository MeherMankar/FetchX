//! Output helpers — keeps JSON/human output consistent.
//!
//! Rule: when `--json` is active, **only** valid JSON goes to stdout.
//! Human text and errors always go to stderr.

use serde::Serialize;
use serde_json::Value;

/// Print a success JSON envelope to stdout.
pub fn print_json_ok<T: Serialize>(payload: &T) {
    match serde_json::to_string(payload) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("JSON serialization error: {e}"),
    }
}

/// Print an error JSON envelope to stderr (never stdout).
pub fn print_json_error(code: &str, message: &str) {
    let v = serde_json::json!({ "ok": false, "error": { "code": code, "message": message } });
    eprintln!("{v}");
}

/// Print a human-readable line to stdout.
pub fn print_human(message: &str) {
    println!("{message}");
}

/// Unified error output: JSON to stderr when json=true, else plain text to stderr.
pub fn print_error(json: bool, message: &str) {
    if json {
        print_json_error("error", message);
    } else {
        eprintln!("error: {message}");
    }
}

/// Format bytes as a human-readable string.
pub fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format speed (bytes/sec) or return "—".
pub fn fmt_speed(bps: Option<u64>) -> String {
    bps.map(|b| format!("{}/s", fmt_bytes(b)))
        .unwrap_or_else(|| "—".to_owned())
}

/// Format ETA in seconds as mm:ss or hh:mm:ss.
pub fn fmt_eta(secs: Option<u64>) -> String {
    let Some(s) = secs else {
        return "—".to_owned();
    };
    if s >= 3600 {
        format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Format percentage from basis points (10000 = 100%).
pub fn fmt_pct(bp: Option<u16>) -> String {
    bp.map(|b| format!("{:.1}%", b as f64 / 100.0))
        .unwrap_or_else(|| "—".to_owned())
}

/// Truncate a string to at most `n` chars, appending "…" if truncated.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let end: usize = s
            .char_indices()
            .nth(n - 1)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

/// Simple table printer. `widths` is the max width of each column.
pub struct Table {
    headers: Vec<String>,
    widths: Vec<usize>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: Vec<&str>) -> Self {
        let widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        Self {
            headers: headers.into_iter().map(str::to_owned).collect(),
            widths,
            rows: Vec::new(),
        }
    }

    pub fn add_row(&mut self, row: Vec<String>) {
        for (i, cell) in row.iter().enumerate() {
            if i < self.widths.len() {
                self.widths[i] = self.widths[i].max(cell.len());
            }
        }
        self.rows.push(row);
    }

    pub fn print(&self) {
        let header_line: String = self
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{:width$}", h, width = self.widths[i]))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{header_line}");

        let sep: String = self
            .widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{sep}");

        for row in &self.rows {
            let line: String = row
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let w = self.widths.get(i).copied().unwrap_or(0);
                    format!("{:width$}", cell, width = w)
                })
                .collect::<Vec<_>>()
                .join("  ");
            println!("{line}");
        }
    }
}

/// Build a JSON Value from a flat list of key-value pairs.
#[allow(dead_code)]
pub fn json_obj(pairs: &[(&str, Value)]) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert((*k).to_owned(), v.clone());
    }
    Value::Object(map)
}

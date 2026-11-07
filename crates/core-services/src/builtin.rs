//! Core implementations of FetchX's built-in download enhancements.
//!
//! These features are native parts of the shared core. They must not depend
//! on the plugin API or plugin manager, and are available to every host.

use std::collections::HashMap;

use reqwest::Client;
use serde_json::Value;
use url::Url;

const MAX_TEMPLATE_LENGTH: usize = 512;
const MAX_FILENAME_BYTES: usize = 240;
const MAX_EXTENSION_LENGTH: usize = 24;

#[derive(Clone, Debug, Default)]
pub struct FilenameTemplateContext {
    pub task_id: String,
    pub file_path: String,
    pub source_url: Option<String>,
    pub requested_at: Option<i64>,
    pub created_by: Option<String>,
    pub metadata: HashMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct FilenameTemplateDiagnostic {
    pub severity: &'static str,
    pub code: String,
    pub message: String,
    pub expression: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FilenameTemplateResult {
    pub output: String,
    pub valid: bool,
    pub diagnostics: Vec<FilenameTemplateDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreBuiltinError {
    Http { status: u16, url: String },
    InvalidResponse,
    UnsafeUrl,
}

impl std::fmt::Display for CoreBuiltinError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { status, url } => write!(formatter, "HTTP {status} from {url}"),
            Self::InvalidResponse => formatter.write_str("invalid response"),
            Self::UnsafeUrl => formatter.write_str("unsafe URL"),
        }
    }
}

impl std::error::Error for CoreBuiltinError {}

fn portable_basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn split_name(path: &str) -> (&str, &str) {
    let filename = portable_basename(path);
    let dot = filename.rfind('.');
    match dot {
        Some(dot) if dot > 0 && filename.len() - dot - 1 <= MAX_EXTENSION_LENGTH => {
            filename.split_at(dot)
        }
        _ => (filename, ""),
    }
}

fn replace_tokens(template: &str, context: &FilenameTemplateContext) -> String {
    let (original, extension) = split_name(&context.file_path);
    let url = context
        .source_url
        .as_deref()
        .and_then(|value| Url::parse(value).ok());
    let mut output = template.to_owned();
    let replacements = [
        ("title", original),
        ("filename", portable_basename(&context.file_path)),
        ("extension", extension.trim_start_matches('.')),
        ("taskId", &context.task_id),
        ("fileName", portable_basename(&context.file_path)),
    ];

    for (name, value) in replacements {
        output = output.replace(&format!("{{{{{name}}}}}"), value);
    }
    if let Some(url) = url {
        let host = url.host_str().unwrap_or_default();
        output = output.replace("{{host}}", host);
        output = output.replace("{{domain}}", host.trim_start_matches("www."));
        output = output.replace("{{path}}", &url.path().trim_matches('/').replace('/', "_"));
    } else {
        for token in ["{{host}}", "{{domain}}", "{{path}}"] {
            output = output.replace(token, "");
        }
    }
    output
}

fn sanitise_filename(mut value: String, extension: &str) -> String {
    value = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{202a}'..='\u{202e}' => '_',
            '\0'..='\u{001f}' | '\u{007f}' => '_',
            other => other,
        })
        .collect::<String>()
        .trim_matches(|character: char| character.is_whitespace() || character == '.')
        .to_owned();

    while value.len() > MAX_FILENAME_BYTES {
        value.pop();
    }
    let stem = value.trim();
    if stem.is_empty() {
        return format!("download{extension}");
    }
    format!("{stem}{extension}")
}

pub fn render_filename_template(
    template: &str,
    context: &FilenameTemplateContext,
) -> FilenameTemplateResult {
    let mut diagnostics = Vec::new();
    if template.len() > MAX_TEMPLATE_LENGTH {
        diagnostics.push(FilenameTemplateDiagnostic {
            severity: "error",
            code: "template_too_long".to_owned(),
            message: format!("template exceeds {MAX_TEMPLATE_LENGTH} characters"),
            expression: None,
        });
        return FilenameTemplateResult {
            output: portable_basename(&context.file_path).to_owned(),
            valid: false,
            diagnostics,
        };
    }

    let (_, extension) = split_name(&context.file_path);
    let output = sanitise_filename(replace_tokens(template, context), extension);
    FilenameTemplateResult {
        output,
        valid: true,
        diagnostics,
    }
}

/// Extracts unique HTTP(S) URLs from mixed text.
pub fn extract_download_links(text: &str) -> Vec<String> {
    let text = text.replace("&amp;", "&");
    let mut links = Vec::new();
    let mut remainder = text.as_str();
    while let Some(start) = find_http_start(remainder) {
        let candidate_end = remainder[start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '<' | '>' | '"' | '\'')
            })
            .map(|position| start + position)
            .unwrap_or(remainder.len());
        let mut candidate = remainder[start..candidate_end]
            .trim_end_matches(|character: char| {
                matches!(
                    character,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'
                )
            })
            .to_owned();
        if !candidate.is_empty() {
            candidate = candidate.trim_end_matches('/').to_owned();
            if let Ok(url) = Url::parse(&candidate) {
                if matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && !links.iter().any(|existing: &String| existing == &candidate)
                {
                    links.push(candidate);
                }
            }
        }
        let next = if candidate_end <= start {
            (start + 1).min(remainder.len())
        } else {
            candidate_end
        };
        remainder = &remainder[next..];
    }
    links
}

fn find_http_start(text: &str) -> Option<usize> {
    ["http://", "https://"]
        .into_iter()
        .filter_map(|scheme| text.to_ascii_lowercase().find(scheme))
        .min()
}

/// Returns a safe filename suggested by the URL's final path segment.
pub fn suggest_download_filename(url: &str) -> String {
    let filename = decode_percent_bytes(
        Url::parse(url)
            .ok()
            .and_then(|url| url.path_segments()?.next_back().map(str::to_owned))
            .unwrap_or_else(|| "download".to_owned()),
    )
    .replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_")
    .chars()
    .map(|character| {
        if character.is_control() {
            '_'
        } else {
            character
        }
    })
    .collect::<String>()
    .trim_matches(|character: char| character.is_whitespace() || character == '.')
    .to_owned();
    if filename.is_empty() {
        "download".to_owned()
    } else {
        sanitise_filename(filename, "")
    }
}

pub fn normalize_download_url(url: &str) -> String {
    let trimmed = url.trim();
    let mut parsed = match Url::parse(trimmed) {
        Ok(parsed) => parsed,
        Err(_) => return trimmed.to_owned(),
    };
    let _ = parsed.set_fragment(None);
    if let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) {
        let _ = parsed.set_host(Some(&host));
    }
    parsed.to_string()
}

pub fn is_duplicate_download_url(url: &str, existing_urls: &[String]) -> bool {
    let normalized = normalize_download_url(url);
    existing_urls
        .iter()
        .any(|existing| normalize_download_url(existing) == normalized)
}

pub fn match_download_category(
    filename: &str,
    url: Option<&str>,
    page_url: Option<&str>,
) -> Option<&'static str> {
    let lower_filename = filename.to_ascii_lowercase();
    let lower_url = url.unwrap_or_default().to_ascii_lowercase();
    let lower_page = page_url.unwrap_or_default().to_ascii_lowercase();
    let categories = [
        (
            "Documents",
            &[".pdf", ".docx", ".doc", ".ppt", ".pptx", ".odt", ".odf"][..],
        ),
        (
            "Compressed",
            &[".zip", ".rar", ".7z", ".gz", ".tar", ".tgz", ".bz2", ".xz"][..],
        ),
        ("Music", &[".mp3", ".aac", ".wav", ".ac3"][..]),
        ("Videos", &[".ts", ".mp4", ".mkv", ".webm", ".avi"][..]),
        (
            "Programs",
            &[
                ".exe", ".msi", ".msix", ".deb", ".dmg", ".rpm", ".iso", ".pkg", ".sh", ".py",
            ][..],
        ),
    ];
    for (name, extensions) in categories {
        if extensions
            .iter()
            .any(|extension| lower_filename.ends_with(extension))
            || extensions
                .iter()
                .any(|extension| lower_url.ends_with(extension))
            || extensions
                .iter()
                .any(|extension| lower_page.ends_with(extension))
        {
            return Some(name);
        }
    }
    None
}

fn decode_percent_bytes(value: String) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or(value)
}

fn hex_value(character: u8) -> Option<u8> {
    match character {
        b'0'..=b'9' => Some(character - b'0'),
        b'a'..=b'f' => Some(character - b'a' + 10),
        b'A'..=b'F' => Some(character - b'A' + 10),
        _ => None,
    }
}

/// Finds an archive file linked directly from an HTML download page.
pub fn find_archive_link(html: &str, base_url: &str) -> Option<String> {
    let base = Url::parse(base_url).ok()?;
    let pattern = ["href=\"", "href='"];
    for marker in pattern {
        let mut rest = html;
        while let Some(position) = rest.find(marker) {
            rest = &rest[position + marker.len()..];
            let end = rest.find(['"', '\''])?;
            let candidate = &rest[..end];
            if candidate.rsplit('.').next().is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "zip" | "gz" | "tgz" | "rar" | "7z" | "exe" | "dmg" | "iso" | "pkg"
                )
            }) {
                return base.join(candidate).ok().map(|url| url.to_string());
            }
        }
    }
    None
}

/// Resolves Wikimedia Commons `File:` pages to their direct upload URL.
pub async fn resolve_core_url(
    client: &Client,
    url: &str,
) -> Result<Option<String>, CoreBuiltinError> {
    let parsed = Url::parse(url).map_err(|_| CoreBuiltinError::InvalidResponse)?;
    if !parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("commons.wikimedia.org"))
        || !parsed.path().starts_with("/wiki/File:")
    {
        return Ok(None);
    }

    let title = parsed.path().trim_start_matches("/wiki/");
    let encoded_title = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("titles", title)
        .finish()
        .trim_start_matches("titles=")
        .to_owned();
    let api = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url&titles={encoded_title}"
    );
    let response = client
        .get(api)
        .header(reqwest::header::USER_AGENT, "FetchXURLResolver/1.0")
        .send()
        .await
        .map_err(|_| CoreBuiltinError::InvalidResponse)?;
    if !response.status().is_success() {
        return Err(CoreBuiltinError::Http {
            status: response.status().as_u16(),
            url: url.to_owned(),
        });
    }
    let body = response
        .text()
        .await
        .map_err(|_| CoreBuiltinError::InvalidResponse)?;
    let data: Value = serde_json::from_str(&body).map_err(|_| CoreBuiltinError::InvalidResponse)?;
    let direct = data["query"]["pages"]
        .as_object()
        .and_then(|pages| pages.values().next())
        .and_then(|page| page["imageinfo"].as_array())
        .and_then(|info| info.first())
        .and_then(|info| info["url"].as_str());
    Ok(direct.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls_and_detects_duplicates() {
        assert_eq!(
            normalize_download_url("HTTPS://Example.COM/file.zip#part"),
            "https://example.com/file.zip"
        );
        assert!(is_duplicate_download_url(
            "https://example.com/file.zip#other",
            &["HTTPS://EXAMPLE.COM/file.zip".to_owned()]
        ));
    }

    #[test]
    fn matches_xdm_style_download_categories() {
        assert_eq!(
            match_download_category("report.pdf", None, None),
            Some("Documents")
        );
        assert_eq!(
            match_download_category("release", Some("https://example.com/thing.zip"), None),
            Some("Compressed")
        );
    }

    #[test]
    fn extracts_and_deduplicates_mixed_download_links() {
        let links = extract_download_links(
            "Files: https://example.com/a.zip, and https://example.com/a.zip\nhttps://example.org/b.bin.",
        );
        assert_eq!(
            links,
            vec![
                "https://example.com/a.zip".to_owned(),
                "https://example.org/b.bin".to_owned(),
            ]
        );
    }

    #[test]
    fn suggests_safe_download_filename() {
        assert_eq!(
            suggest_download_filename("https://example.com/files/report%20final.zip?token=x"),
            "report final.zip"
        );
        assert_eq!(
            suggest_download_filename("https://example.com/files/<bad>.zip"),
            "_bad_.zip"
        );
    }

    #[test]
    fn filename_template_preserves_extension() {
        let context = FilenameTemplateContext {
            task_id: "task-1".into(),
            file_path: "/downloads/report.final.zip".into(),
            source_url: Some("https://example.com/files/report".into()),
            ..Default::default()
        };
        let result = render_filename_template("{{taskId}}-{{title}}", &context);
        assert!(result.valid);
        assert_eq!(result.output, "task-1-report.final.zip");
    }

    #[test]
    fn filename_template_sanitises_platform_characters() {
        let context = FilenameTemplateContext {
            file_path: "video.mp4".into(),
            ..Default::default()
        };
        let result = render_filename_template("bad: name?.mp4", &context);
        assert_eq!(result.output, "bad_ name_.mp4.mp4");
    }

    #[test]
    fn archive_link_resolves_relative_url() {
        let html = r#"<a href="../files/demo.zip">Download</a>"#;
        assert_eq!(
            find_archive_link(html, "https://example.com/downloads/page"),
            Some("https://example.com/files/demo.zip".to_owned())
        );
    }
}

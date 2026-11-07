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

//! Bounded in-process runner for yt-dlp's YouTube JS challenge solver.
//!
//! The embedded EJS 0.8.0 bundles are hash-matched to the authorized
//! yt-dlp fork revision recorded in `docs/research/NATIVE_YTDLP_COVERAGE.md`.

use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::extractor::ExtractionError;

const SOLVER_LIBRARY: &str = include_str!("youtube_js_assets/yt.solver.lib.min.js");
const SOLVER_CORE: &str = include_str!("youtube_js_assets/yt.solver.core.min.js");
const MAX_PLAYER_SCRIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHALLENGES: usize = 128;
const MAX_CHALLENGE_BYTES: usize = 4096;
const MAX_RUNTIME_MEMORY: usize = 128 * 1024 * 1024;
const MAX_RUNTIME_STACK: usize = 1024 * 1024;
const MAX_SOLVE_TIME: Duration = Duration::from_secs(5);
static SOLVER_SEMAPHORE: Semaphore = Semaphore::const_new(2);

#[derive(Default)]
pub(crate) struct ChallengeRequest {
    pub n: Vec<String>,
    pub signatures: Vec<String>,
}

#[derive(Default)]
pub(crate) struct ChallengeResults {
    pub n: std::collections::HashMap<String, String>,
    pub signatures: std::collections::HashMap<String, String>,
}

pub(crate) async fn solve_challenges(
    player_script: String,
    request: ChallengeRequest,
) -> Result<ChallengeResults, ExtractionError> {
    if player_script.len() > MAX_PLAYER_SCRIPT_BYTES
        || request.n.len().saturating_add(request.signatures.len()) > MAX_CHALLENGES
        || request
            .n
            .iter()
            .chain(&request.signatures)
            .any(|challenge| challenge.is_empty() || challenge.len() > MAX_CHALLENGE_BYTES)
    {
        return Err(ExtractionError::UnsupportedUrl);
    }

    let mut challenge_types = Vec::new();
    let mut requests = Vec::new();
    if !request.signatures.is_empty() {
        challenge_types.push(ChallengeType::Signature);
        requests.push(json!({"type": "sig", "challenges": request.signatures}));
    }
    if !request.n.is_empty() {
        challenge_types.push(ChallengeType::N);
        requests.push(json!({"type": "n", "challenges": request.n}));
    }
    if requests.is_empty() {
        return Ok(ChallengeResults::default());
    }

    let payload = json!({
        "type": "player",
        "player": player_script,
        "output_preprocessed": false,
        "requests": requests
    });
    let payload = serde_json::to_string(&payload).map_err(|_| ExtractionError::InvalidOutput)?;
    let source = format!(
        "{SOLVER_LIBRARY}\nObject.assign(globalThis, lib);\n{SOLVER_CORE}\nJSON.stringify(jsc({payload}));"
    );
    let permit = SOLVER_SEMAPHORE
        .acquire()
        .await
        .map_err(|_| ExtractionError::BackendError {
            message: "YouTube challenge solver is unavailable".to_owned(),
        })?;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        evaluate_challenges(source, challenge_types)
    })
    .await
    .map_err(|_| ExtractionError::BackendError {
        message: "YouTube challenge solver failed".to_owned(),
    })?
}

#[derive(Clone, Copy)]
enum ChallengeType {
    Signature,
    N,
}

fn evaluate_challenges(
    source: String,
    request_types: Vec<ChallengeType>,
) -> Result<ChallengeResults, ExtractionError> {
    let runtime = Runtime::new().map_err(|_| ExtractionError::UnsupportedUrl)?;
    runtime.set_memory_limit(MAX_RUNTIME_MEMORY);
    runtime.set_max_stack_size(MAX_RUNTIME_STACK);
    let deadline = Instant::now() + MAX_SOLVE_TIME;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime).map_err(|_| ExtractionError::UnsupportedUrl)?;
    let output = context
        .with(|ctx| ctx.eval::<String, _>(source))
        .map_err(|_| ExtractionError::UnsupportedUrl)?;
    let output: Value =
        serde_json::from_str(&output).map_err(|_| ExtractionError::UnsupportedUrl)?;
    if output.get("type").and_then(Value::as_str) != Some("result") {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let responses = output
        .get("responses")
        .and_then(Value::as_array)
        .filter(|responses| responses.len() == request_types.len())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let mut results = ChallengeResults::default();
    for (response, challenge_type) in responses.iter().zip(request_types) {
        if response.get("type").and_then(Value::as_str) != Some("result") {
            return Err(ExtractionError::UnsupportedUrl);
        }
        let data = response
            .get("data")
            .and_then(Value::as_object)
            .ok_or(ExtractionError::UnsupportedUrl)?;
        let destination = match challenge_type {
            ChallengeType::Signature => &mut results.signatures,
            ChallengeType::N => &mut results.n,
        };
        for (challenge, output) in data {
            let output = output.as_str().ok_or(ExtractionError::UnsupportedUrl)?;
            if output.is_empty() || output.len() > MAX_CHALLENGE_BYTES {
                return Err(ExtractionError::UnsupportedUrl);
            }
            destination.insert(challenge.clone(), output.to_owned());
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER_SCRIPT: &str = r#"
        (function() {
            function Link() {
                this.values = Object.create(null);
                this.alr = function() {};
            }
            Link.prototype.set = function(key, value) {
                this.values[key] = value;
            };
            Link.prototype.get = function(key) {
                return this.values[key];
            };
            Link.prototype.clone = function() {
                const copy = new Link();
                copy.values = Object.assign(Object.create(null), this.values);
                return copy;
            };
            Link.prototype.transform = function() {
                for (const key of ["s", "n"]) {
                    if (this.values[key]) {
                        this.values[key] = encodeURIComponent(
                            decodeURIComponent(this.values[key]).split("").reverse().join("")
                        );
                    }
                }
            };
            var candidate = function(url, key, signature) {
                const link = new Link();
                if (signature) {
                    link.set(key, signature);
                }
                link.alr("alr", "yes");
                return link;
            };
        }).call(this);
    "#;

    #[tokio::test]
    async fn applies_dynamic_signature_and_n_functions_from_player_script() {
        let result = solve_challenges(
            PLAYER_SCRIPT.to_owned(),
            ChallengeRequest {
                n: vec!["abc123".to_owned()],
                signatures: vec!["sig-456".to_owned()],
            },
        )
        .await
        .expect("player transforms should be discovered");
        assert_eq!(
            result.signatures.get("sig-456").map(String::as_str),
            Some("654-gis")
        );
        assert_eq!(result.n.get("abc123").map(String::as_str), Some("321cba"));
    }

    #[tokio::test]
    async fn rejects_player_scripts_without_supported_transform_functions() {
        let result = solve_challenges(
            "(function() { var x = 1; }).call(this);".to_owned(),
            ChallengeRequest {
                n: vec!["abc".to_owned()],
                signatures: Vec::new(),
            },
        )
        .await;
        assert!(matches!(result, Err(ExtractionError::UnsupportedUrl)));
    }

    #[tokio::test]
    async fn solves_only_the_challenge_type_requested() {
        let result = solve_challenges(
            PLAYER_SCRIPT.to_owned(),
            ChallengeRequest {
                n: vec!["abc123".to_owned()],
                signatures: Vec::new(),
            },
        )
        .await
        .expect("n transform should not require a signature transform request");
        assert_eq!(result.n.get("abc123").map(String::as_str), Some("321cba"));
        assert!(result.signatures.is_empty());
    }

    #[tokio::test]
    async fn rejects_challenges_outside_the_resource_budget() {
        let result = solve_challenges(
            PLAYER_SCRIPT.to_owned(),
            ChallengeRequest {
                n: vec!["x".repeat(MAX_CHALLENGE_BYTES + 1)],
                signatures: Vec::new(),
            },
        )
        .await;
        assert!(matches!(result, Err(ExtractionError::UnsupportedUrl)));
    }
}

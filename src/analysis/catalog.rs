//! Live model catalog: which vision models each provider serves right now.
//!
//! Read-only metadata routes only (`/api/tags` + `/api/show` on Ollama,
//! `models.list` on Gemini). Nothing here sends a generation request. The
//! response parsing is pure and unit-tested; the HTTP paths are thin.

use crate::contracts::errors::VisionError;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const GEMINI_MODELS_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
/// Upper bound on `models.list` pages followed.
const GEMINI_MAX_PAGES: usize = 20;

/// One vision-capable model served by an Ollama endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaModel {
    pub name: String,
    /// Whether `thinking` is among the model's capabilities.
    pub thinking: bool,
    pub capabilities: Vec<String>,
}

/// The vision models of an Ollama endpoint, plus the models whose
/// capabilities could not be read.
#[derive(Debug, Clone, Serialize)]
pub struct OllamaCatalog {
    pub models: Vec<OllamaModel>,
    pub unreadable: Vec<String>,
}

/// One Gemini model that supports `generateContent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeminiModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_token_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_token_limit: Option<u64>,
    /// Whether the API reports the model as a thinking model.
    pub thinking: bool,
}

fn client(timeout_seconds: u64) -> Result<reqwest::Client, VisionError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds.max(1)))
        .build()
        .map_err(|e| VisionError::Unavailable(e.to_string()))
}

fn invalid_json(what: &str, e: serde_json::Error) -> VisionError {
    VisionError::ApiError {
        message: format!("invalid JSON from {what}: {e}"),
        status_code: None,
    }
}

/// Model names from an Ollama `/api/tags` body.
pub fn parse_ollama_tags(body: &str) -> Result<Vec<String>, VisionError> {
    let v: Value = serde_json::from_str(body).map_err(|e| invalid_json("Ollama /api/tags", e))?;
    let models = v.get("models").and_then(Value::as_array).ok_or_else(|| VisionError::ApiError {
        message: "Ollama /api/tags carried no 'models' array".to_string(),
        status_code: None,
    })?;
    Ok(models
        .iter()
        .filter_map(|m| m.get("name").or_else(|| m.get("model")).and_then(Value::as_str))
        .map(str::to_string)
        .collect())
}

/// The model described by an Ollama `/api/show` body, if it has the `vision`
/// capability. `Err` when the body cannot be read.
pub fn parse_ollama_show(name: &str, body: &str) -> Result<Option<OllamaModel>, VisionError> {
    let v: Value = serde_json::from_str(body).map_err(|e| invalid_json("Ollama /api/show", e))?;
    let capabilities: Vec<String> = v
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|caps| caps.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if !capabilities.iter().any(|c| c == "vision") {
        return Ok(None);
    }
    Ok(Some(OllamaModel {
        name: name.to_string(),
        thinking: capabilities.iter().any(|c| c == "thinking"),
        capabilities,
    }))
}

/// List the vision-capable models of the Ollama endpoint at `base_url`.
pub async fn list_ollama_vision_models(
    base_url: &str,
    timeout_seconds: u64,
) -> Result<OllamaCatalog, VisionError> {
    let base = base_url.trim_end_matches('/');
    let client = client(timeout_seconds)?;
    let unreachable =
        |e: reqwest::Error| VisionError::Unavailable(format!("cannot reach Ollama at {base}: {e}"));

    let resp = client.get(format!("{base}/api/tags")).send().await.map_err(unreachable)?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(unreachable)?;
    if !(200..300).contains(&status) {
        return Err(VisionError::ApiError {
            message: format!("{base}/api/tags: {}", body.chars().take(300).collect::<String>()),
            status_code: Some(status),
        });
    }

    let mut catalog = OllamaCatalog { models: Vec::new(), unreadable: Vec::new() };
    for name in parse_ollama_tags(&body)? {
        let shown = client
            .post(format!("{base}/api/show"))
            .json(&serde_json::json!({ "model": name }))
            .send()
            .await;
        let body = match shown {
            Ok(r) if r.status().is_success() => r.text().await.ok(),
            _ => None,
        };
        match body.map(|b| parse_ollama_show(&name, &b)) {
            Some(Ok(Some(model))) => catalog.models.push(model),
            Some(Ok(None)) => {}
            Some(Err(_)) | None => catalog.unreadable.push(name),
        }
    }
    catalog.models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(catalog)
}

/// The `generateContent` models of one `models.list` page, and the token of
/// the next page if there is one.
pub fn parse_gemini_models(body: &str) -> Result<(Vec<GeminiModel>, Option<String>), VisionError> {
    let v: Value = serde_json::from_str(body).map_err(|e| invalid_json("Gemini models.list", e))?;
    if let Some(err) = v.get("error") {
        return Err(VisionError::ApiError {
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown Gemini error")
                .to_string(),
            status_code: err.get("code").and_then(Value::as_u64).map(|c| c as u16),
        });
    }
    let models = v
        .get("models")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(Value::as_array)
                        .is_some_and(|methods| methods.iter().any(|x| x == "generateContent"))
                })
                .filter_map(|m| {
                    let name = m.get("name").and_then(Value::as_str)?;
                    Some(GeminiModel {
                        name: name.trim_start_matches("models/").to_string(),
                        display_name: m
                            .get("displayName")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        input_token_limit: m.get("inputTokenLimit").and_then(Value::as_u64),
                        output_token_limit: m.get("outputTokenLimit").and_then(Value::as_u64),
                        thinking: m.get("thinking").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let next = v
        .get("nextPageToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    Ok((models, next))
}

/// The version number that follows the family in a model name
/// (`gemini-3.8-flash` -> `[3, 8]`), or `None` for a name without one there
/// (`gemini-flash-latest`). Later numbers, such as a preview date, are not a
/// version.
fn name_version(name: &str) -> Option<Vec<u32>> {
    let part = name.split('-').nth(1)?;
    let nums: Option<Vec<u32>> = part.split('.').map(|n| n.parse().ok()).collect();
    nums.filter(|n| !n.is_empty())
}

/// Order models newest first. `models.list` reports no release date, so the
/// version number in the name stands in for it: highest version first, with
/// the names that carry no version (the `-latest` aliases, which track the
/// newest model) ahead of them.
pub fn sort_newest_first(models: &mut [GeminiModel]) {
    models.sort_by(|a, b| {
        let (va, vb) = (name_version(&a.name), name_version(&b.name));
        match (va, vb) {
            (None, None) => a.name.cmp(&b.name),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.name.cmp(&b.name)),
        }
    });
}

/// List the Gemini models that support `generateContent`, newest first.
pub async fn list_gemini_models(
    api_key: &str,
    timeout_seconds: u64,
) -> Result<Vec<GeminiModel>, VisionError> {
    let client = client(timeout_seconds)?;
    // The key travels in a header, so a transport error carries no credential.
    let failed = |e: reqwest::Error| {
        VisionError::NetworkError(format!("Gemini models.list: {}", e.without_url()))
    };
    let mut all = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..GEMINI_MAX_PAGES {
        let mut req = client
            .get(GEMINI_MODELS_URL)
            .header("x-goog-api-key", api_key)
            .query(&[("pageSize", "1000")]);
        if let Some(token) = &page_token {
            req = req.query(&[("pageToken", token.as_str())]);
        }
        let resp = req.send().await.map_err(failed)?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(failed)?;
        let (models, next) = parse_gemini_models(&body).map_err(|e| match e {
            VisionError::ApiError { message, status_code: None } => VisionError::ApiError {
                message,
                status_code: Some(status),
            },
            other => other,
        })?;
        if !(200..300).contains(&status) {
            return Err(VisionError::ApiError {
                message: body.chars().take(300).collect(),
                status_code: Some(status),
            });
        }
        all.extend(models);
        page_token = next;
        if page_token.is_none() {
            break;
        }
    }
    sort_newest_first(&mut all);
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_names_from_tags() {
        let body = r#"{"models":[{"name":"minimax-m3:cloud","size":1},{"model":"llava:7b"}]}"#;
        assert_eq!(parse_ollama_tags(body).unwrap(), vec!["minimax-m3:cloud", "llava:7b"]);
        assert!(parse_ollama_tags(r#"{"nope":1}"#).is_err());
        assert!(parse_ollama_tags("not json").is_err());
    }

    #[test]
    fn keeps_only_vision_models_and_reports_thinking() {
        let vision = r#"{"capabilities":["completion","vision","thinking"]}"#;
        let model = parse_ollama_show("qwen3-vl:cloud", vision).unwrap().unwrap();
        assert_eq!(model.name, "qwen3-vl:cloud");
        assert!(model.thinking);

        let plain = r#"{"capabilities":["completion","vision"]}"#;
        assert!(!parse_ollama_show("llava", plain).unwrap().unwrap().thinking);

        let text_only = r#"{"capabilities":["completion","tools"]}"#;
        assert_eq!(parse_ollama_show("gpt-oss", text_only).unwrap(), None);
        assert_eq!(parse_ollama_show("old", r#"{"details":{}}"#).unwrap(), None);
        assert!(parse_ollama_show("bad", "<html>").is_err());
    }

    #[test]
    fn keeps_only_generate_content_models() {
        let body = r#"{
            "models":[
              {"name":"models/gemini-3.8-flash","displayName":"Gemini 3.8 Flash",
               "inputTokenLimit":1048576,"outputTokenLimit":65536,"thinking":true,
               "supportedGenerationMethods":["generateContent","countTokens"]},
              {"name":"models/embedding-9","supportedGenerationMethods":["embedContent"]}
            ],
            "nextPageToken":"abc"
        }"#;
        let (models, next) = parse_gemini_models(body).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "gemini-3.8-flash");
        assert_eq!(models[0].output_token_limit, Some(65536));
        assert!(models[0].thinking);
        assert_eq!(next.as_deref(), Some("abc"));
    }

    #[test]
    fn a_gemini_error_body_is_an_error() {
        let err = parse_gemini_models(r#"{"error":{"code":403,"message":"denied"}}"#).unwrap_err();
        assert!(matches!(err, VisionError::ApiError { status_code: Some(403), .. }));
    }

    #[test]
    fn sorts_highest_version_first_with_aliases_leading() {
        let named = |name: &str| GeminiModel {
            name: name.to_string(),
            display_name: None,
            input_token_limit: None,
            output_token_limit: None,
            thinking: false,
        };
        let mut models = vec![
            named("gemini-3.1-pro-preview"),
            named("gemini-3.8-flash"),
            named("gemini-flash-latest"),
            named("gemini-2.5-flash"),
            named("gemini-3.10-flash"),
            named("deep-research-preview-04-2026"),
        ];
        sort_newest_first(&mut models);
        let order: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            order,
            [
                "deep-research-preview-04-2026",
                "gemini-flash-latest",
                "gemini-3.10-flash",
                "gemini-3.8-flash",
                "gemini-3.1-pro-preview",
                "gemini-2.5-flash"
            ]
        );
    }
}

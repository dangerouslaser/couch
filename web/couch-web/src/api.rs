//! Talking to `couch-confd`.
//!
//! Every mutating call answers with the whole config, so this module has one
//! return type and the app has one piece of state. The alternative - a local
//! copy patched from partial responses - is where an editor starts showing a
//! room that the server deleted.
//!
//! There is no base URL. The bundle is served by the daemon it talks to, so
//! same-origin relative paths are correct by construction and there is no CORS
//! to configure. `trunk serve` proxies `/api` to a local daemon for the dev
//! loop; see `Trunk.toml`.

use couch_model::Config;
use gloo_net::http::Request;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub message: String,
}

impl ApiError {
    fn new(message: impl Into<String>) -> ApiError {
        ApiError { message: message.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub async fn load() -> Result<Config, ApiError> {
    let response = Request::get("/api/config")
        .send()
        .await
        .map_err(|e| ApiError::new(format!("cannot reach the remote: {e}")))?;
    parse(response).await
}

// Paths and bodies are taken by value rather than by reference. These futures
// are handed to `spawn_local`, so they have to be `'static`; borrowing a
// `format!` temporary from the call site would not survive the first await.
pub async fn post(path: impl Into<String>, body: impl Serialize) -> Result<Config, ApiError> {
    let path = path.into();
    send(Request::post(&path), Some(body)).await
}

pub async fn put(path: impl Into<String>, body: impl Serialize) -> Result<Config, ApiError> {
    let path = path.into();
    send(Request::put(&path), Some(body)).await
}

pub async fn delete(path: impl Into<String>) -> Result<Config, ApiError> {
    let path = path.into();
    send(Request::delete(&path), None::<()>).await
}

async fn send(
    builder: gloo_net::http::RequestBuilder,
    body: Option<impl Serialize>,
) -> Result<Config, ApiError> {
    let request = match body {
        Some(value) => builder
            .json(&value)
            .map_err(|e| ApiError::new(format!("cannot encode the request: {e}")))?,
        None => builder
            .build()
            .map_err(|e| ApiError::new(format!("cannot build the request: {e}")))?,
    };
    let response = request
        .send()
        .await
        .map_err(|e| ApiError::new(format!("cannot reach the remote: {e}")))?;
    parse(response).await
}

async fn parse(response: gloo_net::http::Response) -> Result<Config, ApiError> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| ApiError::new(format!("truncated response: {e}")))?;

    if (200..300).contains(&status) {
        return serde_json::from_str(&text)
            .map_err(|e| ApiError::new(format!("the remote sent something unreadable: {e}")));
    }

    // The daemon's errors carry a message and, for a rejected edit, the list of
    // things wrong with it. Surface those verbatim: they name a field, which is
    // the only thing that helps a user fix it.
    Err(ApiError::new(match serde_json::from_str::<Value>(&text) {
        Ok(value) => {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the remote refused the change")
                .to_string();
            match value.get("problems").and_then(Value::as_array) {
                Some(problems) if !problems.is_empty() => {
                    let detail = problems
                        .iter()
                        .filter_map(|p| {
                            Some(format!(
                                "{}: {}",
                                p.get("at")?.as_str()?,
                                p.get("message")?.as_str()?
                            ))
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!("{message} ({detail})")
                }
                _ => message,
            }
        }
        Err(_) => format!("the remote answered {status}"),
    }))
}

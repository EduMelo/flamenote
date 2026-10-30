//! Cliente mínimo para APIs compatíveis com OpenAI (`/v1/chat/completions`).

use crate::config::LlmConfig;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

const MAX_TOKENS: u32 = 4096;
const TIMEOUT: Duration = Duration::from_secs(120);

pub fn complete(llm: &LlmConfig, prompt: &str, selection: &str) -> Result<String, String> {
    complete_with(llm, prompt, selection, MAX_TOKENS)
}

fn complete_with(
    llm: &LlmConfig,
    prompt: &str,
    selection: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let base_url = llm.base_url.trim().trim_end_matches('/');
    let model = llm.model.trim();
    let api_key = llm.api_key.trim();

    if base_url.is_empty() {
        return Err("Defina a base URL do servidor LLM nas configurações.".into());
    }
    if model.is_empty() {
        return Err("Defina o modelo do servidor LLM nas configurações.".into());
    }

    let url = endpoint(base_url);

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": prompt },
            { "role": "user", "content": selection }
        ],
        "stream": false,
        "max_tokens": max_tokens,
    });

    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let mut request = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");

    if !api_key.is_empty() {
        request = request.set("Authorization", &format!("Bearer {api_key}"));
    }

    match request.send_json(body) {
        Ok(response) => {
            let body_str = response
                .into_string()
                .map_err(|err| format!("Não li a resposta do servidor LLM: {err}"))?;
            parse_response(body_str)
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!(
                "Servidor LLM respondeu HTTP {status}: {}",
                error_message(&body)
            ))
        }
        Err(err) => Err(format!("Não falei com o servidor LLM: {err}")),
    }
}

pub fn endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: Option<String>,
}

fn parse_response(body: String) -> Result<String, String> {
    let response: ChatResponse = serde_json::from_str(&body)
        .map_err(|err| format!("Resposta inválida do servidor LLM: {err}"))?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "O servidor LLM respondeu sem escolhas.".to_string())?;
    if choice.finish_reason.as_deref() == Some("length") {
        return Err(
            "A resposta do servidor LLM foi truncada pelo limite de tokens. Tente uma seleção menor."
                .into(),
        );
    }
    let text = choice.message.content.unwrap_or_default();
    if text.trim().is_empty() {
        return Err("O servidor LLM respondeu sem texto.".into());
    }
    Ok(text)
}

fn error_message(body: &str) -> String {
    if let Ok(envelope) = serde_json::from_str::<ErrorEnvelope>(body) {
        if let Some(message) = envelope.error.message {
            return message;
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "sem detalhes".into()
    } else {
        trimmed.chars().take(300).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_accepts_host_v1_or_full_path() {
        assert_eq!(
            endpoint("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint("http://localhost:11434/"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://router.example/v1"),
            "https://router.example/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://router.example/v1/chat/completions"),
            "https://router.example/v1/chat/completions"
        );
    }

    #[test]
    fn reads_openai_chat_content_and_errors() {
        let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#;
        assert_eq!(parse_response(body.into()).unwrap(), "ok");
        let error = r#"{"error":{"message":"modelo ausente"}}"#;
        assert_eq!(error_message(error), "modelo ausente");
        assert!(parse_response(r#"{"choices":[]}"#.into()).is_err());
    }

    #[test]
    fn rejects_a_response_cut_by_the_token_limit() {
        let body = r#"{"choices":[{"message":{"content":"parcial"},"finish_reason":"length"}]}"#;
        let err = parse_response(body.into()).unwrap_err();
        assert!(err.contains("truncada"));
    }

    #[test]
    #[ignore = "faz chamadas reais usando OLLAMA_HOST e OPENAI_BASE_URL"]
    fn live_ollama_and_router() {
        let Ok(host) = std::env::var("OLLAMA_HOST") else {
            eprintln!("OLLAMA_HOST não definido; pulando Ollama");
            return;
        };
        let model = std::env::var("OLLAMA_MODEL")
            .or_else(|_| std::env::var("OLLAMA_DEFAULT_MODEL"))
            .expect("OLLAMA_MODEL ou OLLAMA_DEFAULT_MODEL");
        let ollama = LlmConfig {
            base_url: host,
            model,
            api_key: String::new(),
        };
        let text = complete_with(&ollama, "Diga apenas a palavra ok.", "teste", 256)
            .expect("resposta do Ollama");
        assert!(!text.trim().is_empty());
        println!("Ollama respondeu: {text:?}");

        let (base_url, model, api_key) = (
            std::env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL"),
            std::env::var("OPENAI_MODEL").expect("OPENAI_MODEL"),
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY"),
        );
        let router = LlmConfig {
            base_url,
            model,
            api_key,
        };
        let text = complete_with(&router, "Diga apenas a palavra ok.", "teste", 256)
            .expect("resposta do 9router");
        assert!(!text.trim().is_empty());
        println!("9router respondeu: {text:?}");
    }
}

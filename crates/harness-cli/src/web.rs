use std::process::Command;

use serde_json::Value;

/// Provider-neutral web search used to enrich any model prompt.
pub fn search(query: &str) -> Result<String, String> {
    let key = std::env::var("HARNESS_WEB_SEARCH_API_KEY").map_err(|_| {
        "HARNESS_WEB_SEARCH_API_KEY não definida; configure uma chave Brave Search".to_owned()
    })?;
    let endpoint = std::env::var("HARNESS_WEB_SEARCH_URL")
        .unwrap_or_else(|_| "https://api.search.brave.com/res/v1/web/search".to_owned());
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", "-G", &endpoint])
        .arg("-H")
        .arg(format!("X-Subscription-Token: {key}"))
        .arg("-H")
        .arg("Accept: application/json")
        .arg("--data-urlencode")
        .arg(format!("q={query}"))
        .arg("--data-urlencode")
        .arg("count=5")
        .output()
        .map_err(|error| format!("não foi possível executar a busca web: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "busca web falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("resposta da busca web inválida: {error}"))?;
    let results = value
        .get("web")
        .and_then(|web| web.get("results"))
        .and_then(Value::as_array)
        .ok_or("busca web não retornou resultados")?;
    if results.is_empty() {
        return Err("busca web não encontrou resultados".to_owned());
    }
    let mut context = format!("Resultados web para: {query}\n\n");
    for (index, result) in results.iter().take(5).enumerate() {
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("sem título");
        let url = result.get("url").and_then(Value::as_str).unwrap_or("");
        let description = result
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        context.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n",
            index + 1,
            title,
            url,
            description
        ));
    }
    Ok(context)
}

pub fn requested(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "pesquis",
        "internet",
        "web",
        "notícia",
        "noticia",
        "cotação",
        "cotacao",
        "atual",
        "hoje",
        "latest",
        "news",
        "search",
    ]
    .iter()
    .any(|term| normalized.contains(term))
}

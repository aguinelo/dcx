use std::process::Command;

/// Provider-neutral web search performed by the dcode itself.
pub fn search(query: &str) -> Result<String, String> {
    let endpoint = std::env::var("DCX_WEB_SEARCH_URL")
        .unwrap_or_else(|_| "https://html.duckduckgo.com/html/".to_owned());
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "20",
            "-A",
            "dcode/0.1",
            "-G",
            &endpoint,
        ])
        .arg("--data-urlencode")
        .arg(format!("q={query}"))
        .output()
        .map_err(|error| format!("não foi possível executar a busca web: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "busca web falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let html = String::from_utf8_lossy(&output.stdout);
    let results = parse_results(&html);
    if results.is_empty() {
        return Err("busca web não encontrou resultados".to_owned());
    }
    let mut context = format!("Resultados web para: {query}\n\n");
    for (index, (title, url, description)) in results.iter().take(5).enumerate() {
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

fn parse_results(html: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let mut cursor = 0;
    while let Some(start) = html[cursor..].find("class=\"result__a\"") {
        let tag_start = cursor + start;
        let Some(href_start) = html[..tag_start].rfind("<a") else {
            break;
        };
        let Some(anchor_end) = html[tag_start..].find('>') else {
            break;
        };
        let anchor_end = tag_start + anchor_end;
        let Some(href_offset) = html[href_start..anchor_end].find("href=\"") else {
            break;
        };
        let href_value = href_start + href_offset + 6;
        let Some(href_end) = html[href_value..].find('"') else {
            break;
        };
        let url = decode_entities(&html[href_value..href_value + href_end]);
        let Some(title_offset) = html[tag_start..].find('>') else {
            break;
        };
        let title_start = tag_start + title_offset + 1;
        let Some(title_end) = html[title_start..].find("</a>") else {
            break;
        };
        let title = decode_entities(&html[title_start..title_start + title_end]);
        let after = title_start + title_end + 4;
        let description = html[after..]
            .find("result__snippet")
            .and_then(|offset| {
                let snippet_start = after + offset;
                let content_start = html[snippet_start..].find('>')? + snippet_start + 1;
                let content_end = html[content_start..].find('<')?;
                Some(decode_entities(
                    &html[content_start..content_start + content_end],
                ))
            })
            .unwrap_or_default();
        results.push((strip_tags(&title), url, strip_tags(&description)));
        cursor = after;
    }
    results
}

fn strip_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_public_search_results() {
        let html = r#"<a rel="nofollow" class="result__a" href="https://example.com">Example <b>title</b></a><a class="result__snippet">A &amp; B</a>"#;
        assert_eq!(
            parse_results(html),
            vec![(
                "Example title".into(),
                "https://example.com".into(),
                "A & B".into()
            )]
        );
    }
}

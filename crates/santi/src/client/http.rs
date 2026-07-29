use anyhow::Result;

use super::*;

pub(super) fn compact_capsule_body(capsule: Capsule<'_>) -> Result<serde_json::Value> {
    let Capsule {
        first,
        last,
        from,
        to,
        summary,
        file: summary_file,
        source,
        reason,
        risk,
        queryability,
        preview: dry,
        soul,
    } = capsule;
    let summary = match summary_file {
        Some(path) => read_summary_file(&path)?,
        None => summary.expect("clap requires summary or summary_file"),
    };
    let mut body = serde_json::json!({
        "first": first,
        "last": last,
        "from": from,
        "to": to,
        "summary": summary,
        "capsule": {
            "source": source,
            "reason": reason,
            "risk": risk,
            "queryability": queryability,
        },
        "dry": dry,
    });
    if let Some(soul) = soul {
        body["soul"] = serde_json::Value::from(soul);
    }
    Ok(body)
}

pub(super) fn urlencoding_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub(super) fn build_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid bearer token")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder.build().context("build http client")
}

pub(super) struct Http<'a> {
    pub(super) client: &'a reqwest::Client,
}

impl Http<'_> {
    pub(super) async fn get(&self, url: &str) -> Result<()> {
        let response = self
            .client
            .get(url)
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        print_json(response).await
    }

    pub(super) async fn post(&self, url: &str, body: Option<serde_json::Value>) -> Result<()> {
        let mut request = self.client.post(url).timeout(TIMEOUT);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        print_json(response).await
    }

    pub(super) async fn delete(&self, url: &str) -> Result<()> {
        let response = self
            .client
            .delete(url)
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("DELETE {url}"))?;
        print_json(response).await
    }

    pub(super) async fn owned(&self, url: &str, soul: &str) -> Result<()> {
        let response = self
            .client
            .get(url)
            .header("x-santi-soul-id", soul)
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        print_json(response).await
    }

    pub(super) async fn act(&self, url: &str, soul: &str) -> Result<()> {
        let response = self
            .client
            .post(url)
            .header("x-santi-soul-id", soul)
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        print_json(response).await
    }

    pub(super) async fn spawn(
        &self,
        url: &str,
        capability: &str,
        body: serde_json::Value,
    ) -> Result<()> {
        let response = self
            .client
            .post(url)
            .header("x-santi-job-capability", capability)
            .json(&body)
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        print_json(response).await
    }

    pub(super) async fn follow(&self, url: &str, format: WatchFormat) -> Result<()> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("request failed with status {status}");
        }
        let mut stream = response.bytes_stream();
        let mut stdout = std::io::stdout();
        match format {
            WatchFormat::Raw => {
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.context("read event stream")?;
                    stdout.write_all(&chunk).context("write event chunk")?;
                    stdout.flush().ok();
                }
            }
            WatchFormat::Filtered => {
                let mut buffer = String::new();
                while let Some((event, data)) = next_sse_frame(&mut stream, &mut buffer).await? {
                    print_watch_line(&mut stdout, &event, &data);
                }
            }
        }
        Ok(())
    }
}

pub(super) async fn print_json(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{text}"),
    }
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    Ok(())
}

pub(super) fn strand_send_body(text: String, soul: Option<&str>) -> serde_json::Value {
    let mut content = serde_json::json!({
        "content": [{ "type": "text", "text": text }]
    });
    if let Some(soul) = soul {
        content["soul"] = serde_json::Value::from(soul);
    }
    content
}

pub(super) fn print_watch_line(stdout: &mut impl std::io::Write, event: &str, data: &str) {
    if let Some(line) = render_watch_event(event, data) {
        writeln!(stdout, "{line}").ok();
        stdout.flush().ok();
    }
}

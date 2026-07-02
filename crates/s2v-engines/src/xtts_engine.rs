use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use s2v_core::Cast;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::engine::Engine;
use crate::process::{ensure_running, terminate_process, EngineProcess, DEFAULT_STARTUP_TIMEOUT};

pub struct XttsEngine {
    name: String,
    url: String,
    client: Arc<Client>,
    speaker_cache: Arc<RwLock<HashSet<String>>>,
    exe_path: Option<String>,
    startup_timeout: Duration,
    process: Mutex<Option<EngineProcess>>,
}

impl XttsEngine {
    pub fn new(name: impl Into<String>, url: impl Into<String>, client: Arc<Client>) -> Self {
        Self::with_exe_path(name, url, client, None)
    }

    pub fn with_exe_path(
        name: impl Into<String>,
        url: impl Into<String>,
        client: Arc<Client>,
        exe_path: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            client,
            speaker_cache: Arc::new(RwLock::new(HashSet::new())),
            exe_path,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            process: Mutex::new(None),
        }
    }

    /// 自動起動の待機時間を設定する（省略時は [`DEFAULT_STARTUP_TIMEOUT`]）。
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    async fn is_alive(&self) -> bool {
        matches!(
            self.client.get(format!("{}/speakers", self.url)).send().await,
            Ok(res) if res.status().is_success()
        )
    }
}

#[async_trait]
impl Engine for XttsEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        ensure_running(&self.name, self.exe_path.as_deref(), self.startup_timeout, &self.process, || self.is_alive()).await?;

        let res = self
            .client
            .get(format!("{}/speakers", self.url))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}: サーバーに接続できません: {}", self.name, e))?;

        if !res.status().is_success() {
            anyhow::bail!("{}: /speakers returned {}", self.name, res.status());
        }
        let speakers: Vec<Value> = res.json().await?;
        let mut cache = self.speaker_cache.write().await;
        cache.clear();
        for s in &speakers {
            if let Some(name) = s["name"].as_str() {
                cache.insert(name.to_string());
            }
        }
        info!("[{}] XTTS 起動確認 OK ({} 話者)", self.name, cache.len());
        Ok(())
    }

    fn terminate(&self) {
        terminate_process(&self.name, &self.process);
    }

    fn is_cast_valid(&self, cast: &Cast) -> bool {
        if let Ok(cache) = self.speaker_cache.try_read() {
            if !cache.is_empty() && !cache.contains(&cast.speaker_name) {
                warn!("[{}] 話者 '{}' がキャッシュに見つかりません", self.name, cast.speaker_name);
            }
        }
        true
    }

    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> anyhow::Result<()> {
        // get_tts_settings → patch → set_tts_settings
        if let Ok(q_res) = self.client.post(format!("{}/get_tts_settings", self.url)).send().await {
            if q_res.status().is_success() {
                if let Ok(mut query) = q_res.json::<Value>().await {
                    if let Value::Object(ref mut map) = query {
                        for (k, v) in &cast.params {
                            if map.contains_key(k) {
                                map.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    let _ = self
                        .client
                        .post(format!("{}/set_tts_settings", self.url))
                        .json(&query)
                        .send()
                        .await;
                }
            }
        }

        let lang = cast.params.get("language").and_then(|v| v.as_str()).unwrap_or("ja");
        let payload = json!({
            "text": text,
            "speaker_name": cast.speaker_name,
            "language": lang,
        });

        let res = self
            .client
            .post(format!("{}/tts_to_audio/", self.url))
            .json(&payload)
            .send()
            .await?;

        if !res.status().is_success() {
            anyhow::bail!("{}: tts_to_audio 失敗: {}", self.name, res.status());
        }
        let bytes = res.bytes().await?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_engine(url: &str) -> XttsEngine {
        XttsEngine::new("xtts", url, Arc::new(Client::new()))
    }

    fn dummy_cast() -> Cast {
        Cast {
            name: "テスト".to_string(),
            speaker_name: "en-us-1".to_string(),
            engine_type: "xtts".to_string(),
            pan: 0.0,
            distance: 1.0,
            volume: 1.0,
            params: std::collections::HashMap::new(),
            height: None,
            height_offset: 0.0,
            appearance: None,
        }
    }

    fn speakers_response() -> Value {
        serde_json::json!([{"name": "en-us-1"}, {"name": "ja-jp-1"}])
    }

    #[tokio::test]
    async fn activate_populates_speaker_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        let cache = engine.speaker_cache.read().await;
        assert!(cache.contains("en-us-1"));
        assert!(cache.contains("ja-jp-1"));
    }

    #[tokio::test]
    async fn activate_fails_when_server_down() {
        let engine = make_engine("http://127.0.0.1:1");
        assert!(engine.activate().await.is_err());
    }

    #[tokio::test]
    async fn synthesize_writes_audio_file() {
        let server = MockServer::start().await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let out_path = tmp.path().to_path_buf();

        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/get_tts_settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/set_tts_settings"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/tts_to_audio/"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"RIFF....".to_vec()))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        engine.synthesize("Hello", &dummy_cast(), &out_path).await.unwrap();

        assert!(out_path.exists());
        assert!(std::fs::metadata(&out_path).unwrap().len() > 0);
    }

    #[tokio::test]
    async fn synthesize_fails_on_tts_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/get_tts_settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/set_tts_settings"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/tts_to_audio/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        let result = engine.synthesize("テスト", &dummy_cast(), Path::new("/tmp/out.wav")).await;
        assert!(result.is_err());
    }

    #[test]
    fn is_cast_valid_returns_true_always() {
        let engine = make_engine("http://localhost:8020");
        let cast = dummy_cast();
        assert!(engine.is_cast_valid(&cast));
    }
}

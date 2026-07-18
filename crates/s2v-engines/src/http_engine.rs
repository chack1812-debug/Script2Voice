use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use s2v_core::Cast;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{info, warn, error};

use crate::engine::Engine;
use crate::process::{ensure_running, terminate_process, EngineProcess, DEFAULT_STARTUP_TIMEOUT};

/// スピーカーキャッシュの型: speaker_name -> style_name -> style_id
type SpeakerCache = HashMap<String, HashMap<String, u32>>;

pub struct HttpEngine {
    name: String,
    url: String,
    client: Arc<Client>,
    cache: Arc<RwLock<SpeakerCache>>,
    exe_path: Option<String>,
    args: Vec<String>,
    startup_timeout: Duration,
    process: Mutex<Option<EngineProcess>>,
}

impl HttpEngine {
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
            cache: Arc::new(RwLock::new(HashMap::new())),
            exe_path,
            args: Vec::new(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            process: Mutex::new(None),
        }
    }

    /// 自動起動コマンドの引数を設定する（省略時は空、`exe_path` を引数なしで起動）。
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// 自動起動の待機時間を設定する（省略時は [`DEFAULT_STARTUP_TIMEOUT`]）。
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    async fn is_alive(&self) -> bool {
        matches!(
            self.client.get(format!("{}/version", self.url)).send().await,
            Ok(res) if res.status().is_success()
        )
    }

    async fn refresh_cache(&self) -> anyhow::Result<()> {
        let res = self.client.get(format!("{}/speakers", self.url)).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("{}: /speakers returned {}", self.name, res.status());
        }
        let speakers: Vec<Value> = res.json().await?;
        let mut cache = self.cache.write().await;
        cache.clear();
        for s in &speakers {
            let speaker_name = s["name"].as_str().unwrap_or("").to_string();
            let styles: HashMap<String, u32> = s["styles"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|st| {
                    let name = st["name"].as_str()?.to_string();
                    let id = st["id"].as_u64()? as u32;
                    Some((name, id))
                })
                .collect();
            cache.insert(speaker_name, styles);
        }
        info!("[{}] スピーカーキャッシュを更新しました ({} 話者)", self.name, cache.len());
        Ok(())
    }

    async fn resolve_style_id(&self, cast: &Cast) -> Option<u32> {
        let cache = self.cache.read().await;
        let styles = cache.get(&cast.speaker_name)?;
        let target = cast.params.get("style").and_then(|v| v.as_str()).unwrap_or("ノーマル");
        if let Some(&id) = styles.get(target) {
            return Some(id);
        }
        // fallback to first style
        if let Some((id, name)) = styles.iter().map(|(n, &id)| (id, n)).next() {
            warn!(
                "[{}] スタイル '{}' 不明のため '{}' を使用します",
                self.name, target, name
            );
            return Some(id);
        }
        None
    }
}

#[async_trait]
impl Engine for HttpEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        ensure_running(&self.name, self.exe_path.as_deref(), &self.args, self.startup_timeout, &self.process, || self.is_alive()).await?;
        info!("[{}] 接続確認 OK", self.name);
        self.refresh_cache().await?;
        Ok(())
    }

    fn terminate(&self) {
        terminate_process(&self.name, &self.process);
    }

    fn is_cast_valid(&self, cast: &Cast) -> bool {
        // キャッシュはブロッキングで読める場合のみ検証（非同期コンテキスト外）
        if let Ok(cache) = self.cache.try_read() {
            if !cache.contains_key(&cast.speaker_name) {
                error!("[{}] 話者 '{}' が見つかりません", self.name, cast.speaker_name);
                return false;
            }
        }
        true
    }

    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> anyhow::Result<()> {
        let style_id = self
            .resolve_style_id(cast)
            .await
            .ok_or_else(|| anyhow::anyhow!("{}: 話者 '{}' のスタイル解決失敗", self.name, cast.speaker_name))?;

        // audio_query
        let q_res = self
            .client
            .post(format!("{}/audio_query", self.url))
            .query(&[("text", text), ("speaker", &style_id.to_string())])
            .send()
            .await?;
        if !q_res.status().is_success() {
            anyhow::bail!("{}: audio_query 失敗: {}", self.name, q_res.status());
        }
        let mut query: Value = q_res.json().await?;

        // cast params をクエリにマージ (query にあるキーのみ)
        if let Value::Object(ref mut map) = query {
            for (k, v) in &cast.params {
                if map.contains_key(k) {
                    if let Some(f) = v.as_f64() {
                        map.insert(k.clone(), json!(f));
                    }
                }
            }
        }

        // synthesis
        let s_res = self
            .client
            .post(format!("{}/synthesis", self.url))
            .query(&[("speaker", style_id.to_string())])
            .json(&query)
            .send()
            .await?;
        if !s_res.status().is_success() {
            anyhow::bail!("{}: synthesis 失敗: {}", self.name, s_res.status());
        }
        let bytes = s_res.bytes().await?;
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
    use std::sync::Arc;
    use serde_json::Value;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_engine(url: &str) -> HttpEngine {
        HttpEngine::new("test", url, Arc::new(Client::new()))
    }

    fn dummy_cast() -> Cast {
        Cast {
            name: "テスト".to_string(),
            speaker_name: "ずんだもん".to_string(),
            engine_type: "voicevox".to_string(),
            pan: 0.0,
            distance: 1.0,
            volume: 1.0,
            params: {
                let mut m = HashMap::new();
                m.insert("style".to_string(), Value::String("ノーマル".to_string()));
                m
            },
            height: None,
            height_offset: 0.0,
            appearance: None,
        }
    }

    fn speakers_response() -> Value {
        serde_json::json!([
            {
                "name": "ずんだもん",
                "styles": [
                    {"name": "ノーマル", "id": 3},
                    {"name": "あまあま", "id": 1}
                ]
            }
        ])
    }

    #[tokio::test]
    async fn activate_succeeds_when_server_up() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0.14.0"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
    }

    #[tokio::test]
    async fn activate_fails_when_server_down() {
        let engine = make_engine("http://127.0.0.1:1");
        assert!(engine.activate().await.is_err());
    }

    #[tokio::test]
    async fn activate_populates_speaker_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0.14.0"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        let cache = engine.cache.read().await;
        assert!(cache.contains_key("ずんだもん"));
        assert_eq!(cache["ずんだもん"]["ノーマル"], 3);
    }

    #[tokio::test]
    async fn synthesize_calls_audio_query_then_synthesis() {
        let server = MockServer::start().await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let out_path = tmp.path().to_path_buf();

        Mock::given(method("GET")).and(path("/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0.14.0"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/audio_query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"speedScale": 1.0})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/synthesis"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"RIFF....".to_vec()))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        engine.synthesize("こんにちは", &dummy_cast(), &out_path).await.unwrap();

        assert!(out_path.exists());
        assert!(std::fs::metadata(&out_path).unwrap().len() > 0);
    }

    #[tokio::test]
    async fn synthesize_fails_when_audio_query_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET")).and(path("/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0.14.0"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/speakers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(speakers_response()))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/audio_query"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server).await;

        let engine = make_engine(&server.uri());
        engine.activate().await.unwrap();
        let result = engine.synthesize("テスト", &dummy_cast(), Path::new("/tmp/out.wav")).await;
        assert!(result.is_err());
    }
}

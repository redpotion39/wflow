use anyhow::{Context as AnyhowContext, Result};
use tokio_stream::Stream;
use tonic::codec::ProstCodec;
use tonic::transport::{Channel, ClientTlsConfig};

use crate::proto::{Request, Response};

pub struct FlowGrpcClient {
    channel: Channel,
}

/// Default Baseten model id used by the Wispr Flow gateway.
pub const DEFAULT_BASETEN_MODEL: &str = "model-v31pl413";

impl FlowGrpcClient {
    /// Connects to the gRPC backend with TLS and strict timeouts.
    pub async fn connect(endpoint_url: &str) -> Result<Self> {
        let tls = ClientTlsConfig::new().with_native_roots();
        let channel = Channel::from_shared(endpoint_url.to_string())
            .context("Invalid endpoint URL")?
            .tls_config(tls)?
            .connect_timeout(std::time::Duration::from_secs(4))
            .timeout(std::time::Duration::from_secs(15))
            .connect()
            .await
            .context("Failed to connect to the gRPC transcription service")?;

        Ok(Self { channel })
    }

    /// Performs bidirectional streaming dictation with Bearer authorization.
    ///
    /// The gateway key is supplied by the caller (config / environment); it is
    /// no longer embedded in the binary.
    pub async fn transcribe_stream<S>(
        &mut self,
        stream: S,
        auth_token: Option<&str>,
        baseten_key: Option<&str>,
        baseten_model: &str,
    ) -> Result<tonic::Streaming<Response>>
    where
        S: Stream<Item = Request> + Send + 'static,
    {
        let mut client = tonic::client::Grpc::new(self.channel.clone());
        client
            .ready()
            .await
            .context("Service channel was not ready")?;

        let path = http::uri::PathAndQuery::from_static(
            "/flow_api.v1.TranscriptionService/TranscribeStream",
        );

        let codec = ProstCodec::<Request, Response>::default();

        let mut request = tonic::Request::new(stream);
        if let Some(token) = auth_token.filter(|t| !t.trim().is_empty()) {
            let auth_val = if token.starts_with("Bearer ") {
                token.to_string()
            } else {
                format!("Bearer {}", token.trim())
            };
            if let Ok(meta_val) = auth_val.parse() {
                request.metadata_mut().insert("authorization", meta_val);
            }
        }

        // Production routing headers. Only sent when a gateway key is configured.
        if let Some(key) = baseten_key.filter(|k| !k.trim().is_empty()) {
            if let Ok(v) = format!("Api-Key {}", key.trim()).parse() {
                request.metadata_mut().insert("baseten-authorization", v);
            }
            let model = if baseten_model.trim().is_empty() {
                DEFAULT_BASETEN_MODEL
            } else {
                baseten_model.trim()
            };
            if let Ok(v) = model.parse() {
                request.metadata_mut().insert("baseten-model-id", v);
            }
            if let Ok(v) = "production".parse() {
                request.metadata_mut().insert("x-baseten-environment", v);
            }
        } else {
            tracing::warn!("No Baseten gateway key configured; relying on the account JWT only");
        }

        if let Ok(v) = "false".parse() {
            request.metadata_mut().insert("flow-debug", v);
        }
        if let Ok(v) = "false".parse() {
            request.metadata_mut().insert("disable-formatting", v);
        }

        let response = client
            .streaming(request, path, codec)
            .await
            .context("Failed to start TranscribeStream RPC")?;

        Ok(response.into_inner())
    }
}

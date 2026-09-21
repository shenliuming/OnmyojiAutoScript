use http::{HeaderValue, header::AUTHORIZATION};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::client::IntoClientRequest,
};

use crate::config::AgentConfig;

pub type AgentWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, thiserror::Error)]
pub enum WsClientError {
    #[error(transparent)]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error(transparent)]
    InvalidAuthorizationHeader(#[from] http::header::InvalidHeaderValue),
}

pub async fn connect(config: &AgentConfig) -> Result<AgentWebSocket, WsClientError> {
    let mut request = config.server_ws_url.clone().into_client_request()?;
    let authorization = HeaderValue::from_str(&format!("Bearer {}", config.agent_token))?;
    request.headers_mut().insert(AUTHORIZATION, authorization);

    let (socket, _) = connect_async(request).await?;
    Ok(socket)
}

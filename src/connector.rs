use serenity_voice_model::{
    id::{GuildId, UserId},
    payload::{Heartbeat, Identify},
    Event,
};
use tokio::net::TcpStream;
use tokio_native_tls::{native_tls, TlsConnector, TlsStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

use futures_util::{SinkExt, StreamExt};

pub struct BaseConnection {
    endpoint: String,
    server_id: u64,
    session_id: String,
    token: String,
    user_id: u64,
    ws_stream: Option<WebSocketStream<TlsStream<TcpStream>>>,
    pub heartbeat_interval: f64,
}

impl BaseConnection {
    pub fn new(
        endpoint: String,
        server_id: u64,
        session_id: String,
        token: String,
        user_id: u64,
    ) -> Self {
        BaseConnection {
            endpoint,
            server_id,
            session_id,
            token,
            user_id,
            ws_stream: None,
            heartbeat_interval: 1.0,
        }
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        let (ws_stream, _) = {
            let stream = TcpStream::connect((self.endpoint.as_str(), 443)).await?;
            let connector = TlsConnector::from(native_tls::TlsConnector::new()?);
            let stream = connector.connect(&self.endpoint, stream).await?;
            tokio_tungstenite::client_async(format!("wss://{}/?v=4", self.endpoint), stream).await?
        };
        self.ws_stream.replace(ws_stream);
        Ok(())
    }

    pub async fn pull(&mut self) -> anyhow::Result<()> {
        let msg = {
            let (_, mut read) = self.ws_stream.as_mut().unwrap().split();
            let msg = if let Some(Ok(Message::Text(msg))) = read.next().await {
                msg
            } else {
                return Ok(());
            };
            msg
        };
        let event: Event = serde_json::from_str(&msg)?;
        match event {
            Event::Hello(hello) => {
                self.identify().await?;
                self.send_heartbeat().await?;
                self.heartbeat_interval = hello.heartbeat_interval;
            }
            Event::Heartbeat(_) => {
                println!("Sent heartbeat");
                self.send_heartbeat().await?;
            }
            Event::HeartbeatAck(ack) => {
                println!("Ack: {:?}", ack);
            }
            Event::Ready(ready) => {
                println!("Ready: {:?}", ready);
            }
            _ => {
                println!("Unhandled event: {:?}", event);
            }
        }
        Ok(())
    }

    pub async fn send_heartbeat(&mut self) -> anyhow::Result<()> {
        // Get unix epoch time
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let payload = Event::Heartbeat(Heartbeat { nonce: now as u64 });
        println!("{:?}", serde_json::to_string(&payload)?);
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&payload)?))
            .await?;
        Ok(())
    }

    async fn identify(&mut self) -> anyhow::Result<()> {
        let payload = Event::Identify(Identify {
            server_id: GuildId(self.server_id),
            session_id: self.session_id.clone(),
            token: self.token.clone(),
            user_id: UserId(self.user_id),
        });
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&payload)?))
            .await?;
        Ok(())
    }
}

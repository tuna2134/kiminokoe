use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use serenity_voice_model::{
    id::{GuildId, UserId},
    payload::{Heartbeat, Identify, SelectProtocol},
    Event, ProtocolData,
};
use tokio::net::{TcpStream, UdpSocket};
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
    pub socket: UdpSocket,
    ssrc: u32,
}

impl BaseConnection {
    pub async fn new(
        endpoint: String,
        server_id: u64,
        session_id: String,
        token: String,
        user_id: u64,
    ) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(BaseConnection {
            endpoint,
            server_id,
            session_id,
            token,
            user_id,
            ws_stream: None,
            heartbeat_interval: 1.0,
            socket,
            ssrc: 0,
        })
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
                self.socket.connect((ready.ip, ready.port)).await?;
                self.ssrc = ready.ssrc;
                let (ip, port) = self.ip_discovery().await?;
                println!("Now selecting protocol");
                self.select_protocol(ip, port).await?;
                println!("Selected protocol");
            }
            _ => {
                println!("Unhandled event: {:?}", event);
            }
        }
        Ok(())
    }

    async fn ip_discovery(&self) -> anyhow::Result<(String, u16)> {
        let mut buffer = [0u8; 74];
        // Give 0x1 to request the IP discovery
        buffer[0..2].copy_from_slice(&0x1u16.to_be_bytes());
        buffer[2..4].copy_from_slice(&(70 as u16).to_be_bytes());
        buffer[4..8].copy_from_slice(&self.ssrc.to_be_bytes());
        self.socket.send(&buffer).await?;

        let (ip, port) = loop {
            let mut buffer = [0u8; 74];
            let n = self.socket.recv(&mut buffer).await?;
            if n != 74 {
                continue;
            }
            let ip = String::from_utf8_lossy(&buffer[8..72]);
            let port = u16::from_be_bytes([buffer[72], buffer[73]]);
            break (ip.to_string(), port);
        };
        Ok((ip, port))
    }

    pub async fn send_event(&mut self, event: Event) -> anyhow::Result<()> {
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&event)?))
            .await?;
        Ok(())
    }

    async fn select_protocol(&mut self, ip: String, port: u16) -> anyhow::Result<()> {
        println!("{:?}, {}", ip, port);
        let ipaddr: IpAddr = Ipv4Addr::from_str(&ip)?.into();
        let payload = Event::SelectProtocol(SelectProtocol {
            protocol: "udp".to_string(),
            data: ProtocolData {
                address: ipaddr,
                port: port,
                mode: "xsalsa20_poly1305".to_string(),
            },
        });
        println!("Sending select protocol");
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&payload)?))
            .await?;
        Ok(())
    }

    pub async fn send_heartbeat(&mut self) -> anyhow::Result<()> {
        // Get unix epoch time
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let payload = Event::Heartbeat(Heartbeat { nonce: now as u64 });
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

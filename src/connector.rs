use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use crypto_secretbox::aead::generic_array::GenericArray;
use crypto_secretbox::aead::{Aead, AeadMutInPlace, Buffer};
use crypto_secretbox::{Key, KeyInit, XSalsa20Poly1305};
use serenity_voice_model::payload::Speaking;
use serenity_voice_model::SpeakingState;
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
    crypto: Option<XSalsa20Poly1305>,
    sequence: u16,
    timestamp: u32,
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
            crypto: None,
            sequence: 0,
            timestamp: 0,
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
                self.select_protocol(ip, port).await?;
            }
            Event::SessionDescription(session_description) => {
                let key = Key::from_slice(&session_description.secret_key);
                self.crypto.replace(XSalsa20Poly1305::new(key));
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
            let ip_start: usize = 8;
            let ip = if let Some(ip_end) = buffer[ip_start..].iter().position(|&x| x == 0) {
                let ip = std::str::from_utf8(&buffer[ip_start..ip_start + ip_end])
                    .expect("Failed to decode IP")
                    .to_string();
                ip
            } else {
                continue;
            };
            let port = u16::from_be_bytes([buffer[72], buffer[73]]);
            break (ip.to_string(), port);
        };
        Ok((ip, port))
    }

    pub async fn send_audio(&mut self, data: &[u8]) -> anyhow::Result<()> {
        // 1920ずつに分割
        self.sequence += 1;
        let mut buffer = [0u8; 12];
        buffer[0..1].copy_from_slice(&0x80u8.to_be_bytes());
        buffer[1..2].copy_from_slice(&0x78u8.to_be_bytes());
        buffer[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        buffer[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        buffer[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
        let mut nonce = vec![0u8; 24];
        nonce[0..12].copy_from_slice(&buffer[0..12]);
        let mut data = data.to_vec();
        self.crypto
            .as_mut()
            .unwrap()
            .encrypt_in_place(GenericArray::from_slice(&nonce), &[], &mut data)
            .unwrap();
        let mut encrypted = [0u8; 275 + 24 + 12 + 24 + 16 + 12];
        encrypted[0..12].copy_from_slice(&buffer[0..12]);
        encrypted[12..data.len()].copy_from_slice(&data);
        self.socket.send(&encrypted).await?;
        self.timestamp += 1920;
        Ok(())
    }

    pub async fn speaking(&mut self) -> anyhow::Result<()> {
        let payload = Event::Speaking(Speaking {
            speaking: SpeakingState::PRIORITY,
            delay: None,
            ssrc: self.ssrc,
            user_id: None,
        });
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&payload)?))
            .await?;
        Ok(())
    }

    pub async fn send_event(&mut self, event: Event) -> anyhow::Result<()> {
        let (mut write, _) = self.ws_stream.as_mut().unwrap().split();
        write
            .send(Message::Text(serde_json::to_string(&event)?))
            .await?;
        Ok(())
    }

    async fn select_protocol(&mut self, ip: String, port: u16) -> anyhow::Result<()> {
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

use connector::BaseConnection;
use pyo3::prelude::*;
use serenity_voice_model::Event;

use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use flume::{Receiver, Sender};

mod connector;

#[pyclass]
struct VoiceConnector {
    #[pyo3(get, set)]
    token: String,
    #[pyo3(get, set)]
    guild_id: u64,
    #[pyo3(get, set)]
    user_id: u64,
    #[pyo3(get, set)]
    session_id: String,
    #[pyo3(get, set)]
    endpoint: String,
}

#[pymethods]
impl VoiceConnector {
    #[new]
    fn new(server_id: u64, user_id: u64) -> Self {
        VoiceConnector {
            token: "".to_string(),
            guild_id: server_id,
            user_id,
            session_id: "".to_string(),
            endpoint: "".to_string(),
        }
    }

    fn connect<'a>(&'a self, py: Python<'a>) -> anyhow::Result<Bound<'a, PyAny>> {
        let endpoint = self.endpoint.clone();
        let guild_id = self.guild_id;
        let session_id = self.session_id.clone();
        let token = self.token.clone();
        let user_id = self.user_id;
        Ok(pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let connection = Arc::new(Mutex::new(connector::BaseConnection::new(
                    endpoint,
                    guild_id,
                    session_id,
                    token,
                    user_id,
                ).await?));
                let mut connection_lock = connection.lock().await;
                connection_lock.connect().await?;
                connection_lock.pull().await?;
                // let (tx, rx) = tokio::sync::oneshot::channel::<Event>();
                Ok(VoiceConnection::new(Arc::clone(&connection)))
            },
        )?)
    }
}

#[pyclass]
struct VoiceConnection {
    connection: Arc<Mutex<BaseConnection>>,
    rx: Arc<Receiver<Event>>,
    tx: Sender<Event>,
}

impl VoiceConnection {
    pub fn new(connection: Arc<Mutex<BaseConnection>>) -> Self {
        let (tx, rx) = flume::unbounded();
        VoiceConnection { connection, rx: Arc::new(rx), tx }
    }
}

#[pymethods]
impl VoiceConnection {
    fn run<'a>(&'a self) -> anyhow::Result<()> {
        let connection = self.connection.clone();
        let runtime = pyo3_async_runtimes::tokio::get_runtime();
        let rx = self.rx.clone();
        runtime.spawn(async move {
            let mut next_heartbeat = {
                let connection_lock = connection.lock().await;
                Instant::now() + Duration::from_secs_f64((connection_lock.heartbeat_interval) / 1000.0)
            };
            loop {
                let hb = tokio::time::sleep_until(next_heartbeat);
                tokio::select! {
                    _ = hb => {
                        let mut connection_lock = connection.lock().await;
                        connection_lock.send_heartbeat().await.unwrap();
                        next_heartbeat = Instant::now() + Duration::from_secs_f64(connection_lock.heartbeat_interval / 1000.0);
                    }
                    _ = async {
                        let mut connection_lock = connection.lock().await;
                        if let Err(e) = connection_lock.pull().await {
                            eprintln!("Error: {}", e);
                        }
                    } => {}
                    event = rx.recv_async() => {
                        let mut connection_lock = connection.lock().await;
                        connection_lock.send_event(event.unwrap()).await.unwrap();
                    }
                }
            }
        });
        Ok(())
    }
}

/// Formats the sum of two numbers as string.
#[pyfunction]
fn sum_as_string(a: usize, b: usize) -> PyResult<String> {
    Ok((a + b).to_string())
}

/// A Python module implemented in Rust.
#[pymodule]
fn kiminokoe(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sum_as_string, m)?)?;
    m.add_class::<VoiceConnector>()?;
    Ok(())
}

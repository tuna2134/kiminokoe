use connector::BaseConnection;
use pyo3::prelude::*;

use audiopus::{coder::Encoder, Application, Channels, SampleRate};
use flume::{Receiver, Sender};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

mod command;
mod connector;

use command::Command as VoiceCommand;

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
                let connection = Arc::new(Mutex::new(
                    connector::BaseConnection::new(endpoint, guild_id, session_id, token, user_id)
                        .await?,
                ));
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
    rx: Arc<Receiver<VoiceCommand>>,
    tx: Sender<VoiceCommand>,
}

impl VoiceConnection {
    pub fn new(connection: Arc<Mutex<BaseConnection>>) -> Self {
        let (tx, rx) = flume::unbounded();
        VoiceConnection {
            connection,
            rx: Arc::new(rx),
            tx,
        }
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
                    cmd = rx.recv_async() => {
                        let cmd = cmd.unwrap();
                        match cmd {
                            VoiceCommand::Play(data) => {
                                let pcm_samples: &[i16] = unsafe {
                                    assert!(data.len() % 2 == 0);
                                    std::slice::from_raw_parts(
                                        data.as_ptr() as *const i16,
                                        data.len() / 2
                                    )
                                };
                                let mut connection_lock = connection.lock().await;
                                connection_lock.speaking().await.unwrap();
                                let encoder = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Voip).unwrap();
                                for data in pcm_samples.chunks(1920) {
                                    let mut inputs = [0i16; 1920];
                                    inputs[0..data.len()].copy_from_slice(data);
                                    let mut output = [0u8; 1275];
                                    println!("Encoding audio: {:?}", inputs.len());
                                    let size = encoder.encode(&inputs, &mut output).unwrap();
                                    println!("Sending audio: {:?}", &output[0..size]);
                                    connection_lock.send_audio(&output[0..size]).await.unwrap();
                                };
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn play<'a>(&'a self, data: Vec<u8>) -> anyhow::Result<()> {
        self.tx.send(VoiceCommand::Play(data))?;
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

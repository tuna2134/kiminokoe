use connector::BaseConnection;
use pyo3::prelude::*;

use std::sync::Arc;
use tokio::sync::Mutex;

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
        let connection = Arc::new(Mutex::new(connector::BaseConnection::new(
            self.endpoint.clone(),
            self.guild_id,
            self.session_id.clone(),
            self.token.clone(),
            self.user_id,
        )));
        Ok(pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let mut connection_lock = connection.lock().await;
                connection_lock.connect().await?;
                connection_lock.pull().await?;
                // let (tx, rx) = tokio::sync::oneshot::channel::<Event>();
                Ok(VoiceConnection {
                    connection: Arc::clone(&connection),
                })
            },
        )?)
    }
}

#[pyclass]
struct VoiceConnection {
    connection: Arc<Mutex<BaseConnection>>,
}

#[pymethods]
impl VoiceConnection {
    fn run<'a>(&'a self) -> anyhow::Result<()> {
        let connection = self.connection.clone();
        let runtime = pyo3_async_runtimes::tokio::get_runtime();
        runtime.spawn(async move {
            let mut connection_lock = connection.lock().await;
            connection_lock.pull().await.unwrap();
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

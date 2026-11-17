//! The remote mode of the API (API.md §2.1): the operations of the JSON API
//! of a web server, called by a program with an access token.

use std::time::Duration;

use apeiron::{Error, ErrorKind, Progress};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A connection to the JSON API of a web server. Its methods block.
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    runtime: tokio::runtime::Runtime,
}

fn network(e: reqwest::Error) -> Error {
    Error::new(ErrorKind::Io, format!("server: {e}"))
}

impl Client {
    /// A client of the server at `server`, such as `https://nas:8443`,
    /// acting with `token` (made by `apeiron web token add`). `ca` is a PEM
    /// file of certificates to trust besides the system's, such as the
    /// server's own self-signed one.
    pub fn new(server: &str, token: &str, ca: Option<&[u8]>) -> Result<Self, Error> {
        let base = server.trim_end_matches('/').to_owned();
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the server's address starts with https:// or http://",
            ));
        }
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(pem) = ca {
            for cert in reqwest::Certificate::from_pem_bundle(pem)
                .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("certificates: {e}")))?
            {
                builder = builder.add_root_certificate(cert);
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        Ok(Client {
            http: builder.build().map_err(network)?,
            base,
            token: token.trim().to_owned(),
            runtime,
        })
    }

    fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, Error> {
        self.runtime.block_on(async {
            let mut request = self
                .http
                .request(method, format!("{}{path}", self.base))
                .bearer_auth(&self.token);
            if let Some(body) = body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.to_string());
            }
            let response = request.send().await.map_err(network)?;
            let ok = response.status().is_success();
            let status = response.status();
            let bytes = response.bytes().await.map_err(network)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                Error::new(
                    ErrorKind::Io,
                    format!("the server answered {status} without JSON; is it an Apeiron server?"),
                )
            })?;
            if ok {
                Ok(value)
            } else {
                Err(
                    serde_json::from_value::<Error>(value.clone()).unwrap_or_else(|_| {
                        Error::new(
                            ErrorKind::Io,
                            value["message"]
                                .as_str()
                                .map_or_else(|| status.to_string(), str::to_owned),
                        )
                    }),
                )
            }
        })
    }

    /// Runs a short operation, as `apeiron_call` does (SDK.md).
    pub fn call(&self, operation: &str, request: &Value) -> Result<Value, Error> {
        self.send(
            reqwest::Method::POST,
            &format!("/api/call/{operation}"),
            Some(request),
        )
    }

    /// [`Client::call`] with typed request and result, such as
    /// [`apeiron::LogRequest`] and [`apeiron::LogPage`].
    pub fn call_typed<R: Serialize, T: DeserializeOwned>(
        &self,
        operation: &str,
        request: &R,
    ) -> Result<T, Error> {
        let request = serde_json::to_value(request)
            .map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))?;
        serde_json::from_value(self.call(operation, &request)?)
            .map_err(|e| Error::new(ErrorKind::Internal, format!("result: {e}")))
    }

    /// Starts a long operation on the server.
    pub fn start(&self, operation: &str, request: &Value) -> Result<RemoteJob<'_>, Error> {
        let started = self.send(
            reqwest::Method::POST,
            &format!("/api/start/{operation}"),
            Some(request),
        )?;
        let id = started["job"]
            .as_u64()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "the server gave no job"))?;
        Ok(RemoteJob { client: self, id })
    }
}

/// A long operation running on a server.
pub struct RemoteJob<'a> {
    client: &'a Client,
    id: u64,
}

impl RemoteJob<'_> {
    /// Asks the server to stop it.
    pub fn cancel(&self) -> Result<(), Error> {
        self.client
            .send(
                reqwest::Method::POST,
                &format!("/api/jobs/{}/cancel", self.id),
                None,
            )
            .map(drop)
    }

    /// Waits for the result, calling `progress` about four times a second.
    pub fn wait(self, mut progress: impl FnMut(&Progress)) -> Result<Value, Error> {
        loop {
            let state = self.client.send(
                reqwest::Method::GET,
                &format!("/api/jobs/{}", self.id),
                None,
            )?;
            if let Ok(p) = serde_json::from_value::<Progress>(state["progress"].clone()) {
                progress(&p);
            }
            if state["done"] == true {
                if !state["error"].is_null() {
                    return Err(
                        serde_json::from_value(state["error"].clone()).unwrap_or_else(|_| {
                            Error::new(ErrorKind::Internal, state["error"].to_string())
                        }),
                    );
                }
                return Ok(state["result"].clone());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

//! The only way out of the sandbox (PLUGIN.md §3.1): HTTPS requests to the
//! hosts the user granted a plugin; plain HTTP only to this computer.

use wasmtime_wasi_http::{Error, RequestOptions, WasiHttpHooks};

type HttpResult<T> = std::result::Result<T, Error>;

/// Lets requests through to the granted hosts only.
pub(crate) struct Gate {
    /// `host` or `host:port`, lowercase.
    hosts: Vec<String>,
}

impl Gate {
    pub(crate) fn new(hosts: Vec<String>) -> Self {
        Gate {
            hosts: hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect(),
        }
    }

    /// Whether a request to `host`, on `port`, over `https` or not, may go.
    pub(crate) fn allows(&self, host: &str, port: Option<u16>, https: bool) -> bool {
        let host = host.to_ascii_lowercase();
        let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
        if !https && !loopback {
            return false;
        }
        let with_port = port.map(|p| format!("{host}:{p}"));
        self.hosts
            .iter()
            .any(|h| *h == host || with_port.as_deref() == Some(h.as_str()))
    }
}

impl WasiHttpHooks for Gate {
    fn send_request(
        &mut self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
        options: Option<RequestOptions>,
        fut: Box<dyn Future<Output = HttpResult<()>> + Send>,
    ) -> Box<
        dyn Future<
                Output = HttpResult<(
                    http::Response<wasmtime_wasi_http::WasiBody>,
                    Box<dyn Future<Output = HttpResult<()>> + Send>,
                )>,
            > + Send,
    > {
        let uri = request.uri();
        let https = uri.scheme_str() == Some("https");
        let allowed = uri
            .host()
            .is_some_and(|host| self.allows(host, uri.port_u16(), https));
        if !allowed {
            return Box::new(async { Err(Error::HttpRequestDenied) });
        }
        drop(fut);
        // Sent as Wasmtime does without a gate.
        Box::new(async move {
            use http_body_util::BodyExt;
            let (response, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            Ok((
                response.map(BodyExt::boxed_unsync),
                Box::new(io) as Box<dyn Future<Output = HttpResult<()>> + Send>,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Gate;

    #[test]
    fn only_granted_hosts_and_https_pass() {
        let gate = Gate::new(vec![
            "hooks.example.com".to_owned(),
            "127.0.0.1:8080".to_owned(),
        ]);
        assert!(gate.allows("hooks.example.com", None, true));
        assert!(gate.allows("HOOKS.example.com", Some(443), true));
        assert!(!gate.allows("hooks.example.com", None, false));
        assert!(!gate.allows("evil.example.com", None, true));
        assert!(gate.allows("127.0.0.1", Some(8080), false));
        assert!(!gate.allows("127.0.0.1", Some(9090), false));
        assert!(!Gate::new(Vec::new()).allows("hooks.example.com", None, true));
    }
}

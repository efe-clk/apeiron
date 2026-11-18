//! A small blocking HTTP client over wasi:http, shared by the network
//! fixtures of the plugin tests.

use wasi::http::outgoing_handler;
use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};

/// Sends `body` with `method` to `url`; the status and the answer.
pub fn request(method: &str, url: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let (scheme, rest) = url.split_once("://").ok_or("not an address")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let request = OutgoingRequest::new(Fields::new());
    let method = match method {
        "PUT" => Method::Put,
        "POST" => Method::Post,
        "DELETE" => Method::Delete,
        _ => Method::Get,
    };
    let fail = |what: &str| format!("cannot set the {what}");
    request.set_method(&method).map_err(|()| fail("method"))?;
    let scheme = if scheme == "https" { Scheme::Https } else { Scheme::Http };
    request.set_scheme(Some(&scheme)).map_err(|()| fail("scheme"))?;
    request.set_authority(Some(authority)).map_err(|()| fail("host"))?;
    request.set_path_with_query(Some(path)).map_err(|()| fail("path"))?;
    let out = request.body().map_err(|()| fail("body"))?;
    {
        let stream = out.write().map_err(|()| fail("body stream"))?;
        for chunk in body.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(out, None).map_err(|e| format!("{e:?}"))?;
    let future = outgoing_handler::handle(request, None).map_err(|e| format!("{e:?}"))?;
    future.subscribe().block();
    let response = future
        .get()
        .ok_or("no answer")?
        .map_err(|()| "the answer was taken")?
        .map_err(|e| format!("{e:?}"))?;
    let status = response.status();
    let incoming = response.consume().map_err(|()| fail("answer"))?;
    let mut data = Vec::new();
    {
        let stream = incoming.stream().map_err(|()| fail("answer stream"))?;
        loop {
            match stream.blocking_read(1 << 16) {
                Ok(chunk) => data.extend_from_slice(&chunk),
                Err(wasi::io::streams::StreamError::Closed) => break,
                Err(e) => return Err(format!("{e:?}")),
            }
        }
    }
    Ok((status, data))
}

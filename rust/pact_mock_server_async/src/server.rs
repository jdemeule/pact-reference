use pact_matching::models::{Pact, Interaction, Request, OptionalBody, PactSpecification};
use pact_matching::models::matchingrules::*;
use pact_matching::models::generators::*;
use pact_matching::models::parse_query_string;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use log::{log, debug, warn};
use hyper::{Body, Response, Server, Error};
use hyper::service::service_fn;
use futures::future;
use futures::future::Future;
use futures::stream::Stream;

enum MockRequestError {
    InvalidHeaderEncoding,
    RequestBodyError
}

fn extract_path(uri: &hyper::Uri) -> String {
    uri.path_and_query()
        .map(|path_and_query| path_and_query.path())
        .unwrap_or("/")
        .into()
}

fn extract_query_string(uri: &hyper::Uri) -> Option<HashMap<String, Vec<String>>> {
    uri.path_and_query()
        .and_then(|path_and_query| path_and_query.query())
        .and_then(|query| parse_query_string(&query.into()))
}

fn extract_headers(headers: &hyper::HeaderMap) -> Result<Option<HashMap<String, String>>, MockRequestError> {
    if headers.len() > 0 {
        let result: Result<HashMap<String, String>, MockRequestError> = headers.keys()
            .map(|name| -> Result<(String, String), MockRequestError> {
                let values = headers.get_all(name);
                let mut iter = values.iter();

                let first_value = iter.next().unwrap();

                if iter.next().is_some() {
                    warn!("Multiple headers associated with '{}', but only the first is used", name);
                }

                Ok((
                    name.as_str().into(),
                    first_value.to_str()
                        .map_err(|err| MockRequestError::InvalidHeaderEncoding)?
                        .into()
                    )
                )
            })
            .collect();

        result.map(|map| Some(map))
    } else {
        Ok(None)
    }
}

pub fn extract_body(chunk: hyper::Chunk) -> OptionalBody {
    let bytes = chunk.into_bytes();
    if bytes.len() > 0 {
        OptionalBody::Present(bytes.to_vec())
    } else {
        OptionalBody::Empty
    }
}

fn hyper_request_to_pact_request(req: hyper::Request<Body>) -> impl Future<Item = Request, Error = MockRequestError> {
    let method = req.method().to_string();
    let path = extract_path(req.uri());
    let query = extract_query_string(req.uri());
    let headers = extract_headers(req.headers());

    future::done(headers)
        .and_then(move |headers| {
            req.into_body()
                .concat2()
                .map_err(|_| MockRequestError::RequestBodyError)
                .map(|body_chunk| (headers, body_chunk))
        })
        .and_then(|(headers, body_chunk)|
            Ok(Request {
                method: method,
                path: path,
                query: query,
                headers: headers,
                body: extract_body(body_chunk),
                matching_rules: MatchingRules::default(),
                generators: Generators::default()
            })
        )
}

fn handle_request(
    req: hyper::Request<Body>,
    pact: Arc<Pact>,
) -> impl Future<Item = Response<Body>, Error = MockRequestError> {
    debug!("Creating pact request from hyper request");

    hyper_request_to_pact_request(req)
        .map(|req| {
            Response::new(Body::from("Hello World"))
        })
}

fn handle_mock_request_error(result: Result<Response<Body>, MockRequestError>) -> Result<Response<Body>, Error> {
    match result {
        Ok(response) => Ok(response),
        Err(error) => {
            let response = match error {
                MockRequestError::InvalidHeaderEncoding => Response::builder()
                    .status(400)
                    .body(Body::from("Invalid header encoding")),
                MockRequestError::RequestBodyError => Response::builder()
                    .status(500)
                    .body(Body::from("Could not process request body"))
            };
            Ok(response.unwrap())
        }
    }
}

pub fn start(
    id: String,
    pact: Pact,
    port: u16,
    shutdown: impl Future<Item = (), Error = ()>,
) -> (impl Future<Item = (), Error = Error>, u16) {
    let pact = Arc::new(pact);
    let addr = ([0, 0, 0, 0], port).into();

    let server = Server::bind(&addr)
        .serve(move || {
            let pact = pact.clone();
            service_fn(move |req| {
                handle_request(req, pact.clone())
                    .then(handle_mock_request_error)
            })
        });

    let port = server.local_addr().port();

    (server.with_graceful_shutdown(shutdown), port)
}

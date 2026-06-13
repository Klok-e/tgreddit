//! Shared HTTP test server for OAuth + API fixture tests.
//!
//! This module is only compiled under `#[cfg(test)]` and provides a minimal
//! HTTP/1.1 server backed by a `TcpListener` that can serve a sequence of
//! canned responses keyed by request path. Tests use it to point the Reddit
//! OAuth transport at a local socket instead of `oauth.reddit.com`.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

#[derive(Clone)]
pub struct TestResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl TestResponse {
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "application/json".to_owned())],
            body: body.to_owned(),
        }
    }

    pub fn json_with_session(body: &str, loid: &str, session: &str) -> Self {
        Self {
            status: 200,
            headers: vec![
                ("content-type", "application/json".to_owned()),
                ("x-reddit-loid", loid.to_owned()),
                ("x-reddit-session", session.to_owned()),
            ],
            body: body.to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct TestServer {
    base_url: String,
    handle: Option<thread::JoinHandle<Vec<RecordedRequest>>>,
}

impl TestServer {
    /// Start a server that serves `responses_by_path[path]` for each request
    /// and falls back to `default_response` (or 404) when the path is not
    /// configured. The OAuth token endpoint path is always served as
    /// `token_response` if provided.
    pub fn start(responses: TestServerConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let mut requests = Vec::new();
            let mut api_requests_served = 0;
            loop {
                let (stream, _) = match listener.accept() {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let request = read_request(&stream);
                let is_token = request.path == "/auth/v2/oauth/access-token/loid";
                if !is_token {
                    api_requests_served += 1;
                }
                let response = responses.response_for(&request);
                write_response(stream, response.status, &response.headers, &response.body);
                requests.push(request);
                if !is_token && api_requests_served >= responses.stop_after {
                    break;
                }
            }
            requests
        });
        ready_rx.recv().unwrap();
        Self {
            base_url,
            handle: Some(handle),
        }
    }

    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    pub fn join(mut self) -> Vec<RecordedRequest> {
        // Send a request to unblock the accept loop if it's still waiting.
        std::mem::drop(reqwest::get(format!("{}/__shutdown", self.base_url)));
        self.handle.take().unwrap().join().unwrap()
    }
}

pub struct TestServerConfig {
    pub token_response: Option<TestResponse>,
    pub responses_by_path: HashMap<String, TestResponse>,
    pub default_response: Option<TestResponse>,
    pub stop_after: usize,
}

impl TestServerConfig {
    fn response_for(&self, request: &RecordedRequest) -> TestResponse {
        if request.path == "/auth/v2/oauth/access-token/loid"
            && let Some(token) = &self.token_response
        {
            return token.clone();
        }
        let full_key = if request.query.is_empty() {
            request.path.clone()
        } else {
            format!("{}?{}", request.path, request.query)
        };
        if let Some(response) = self.responses_by_path.get(&full_key) {
            return response.clone();
        }
        if let Some(response) = self.responses_by_path.get(&request.path) {
            return response.clone();
        }
        if let Some(default) = &self.default_response {
            return default.clone();
        }
        TestResponse {
            status: 404,
            headers: vec![("content-type", "text/plain".to_owned())],
            body: "not found".to_owned(),
        }
    }
}

fn read_request(stream: &TcpStream) -> RecordedRequest {
    let mut reader = BufReader::new(stream);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).unwrap();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap().to_owned();
    let target = parts.next().unwrap().to_owned();
    let (path, query) = target
        .split_once('?')
        .map(|(path, query)| (path.to_owned(), query.to_owned()))
        .unwrap_or((target, String::new()));

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(": ").unwrap();
        headers.insert(name.to_ascii_lowercase(), value.to_owned());
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).unwrap();
    }

    RecordedRequest {
        method,
        path,
        query,
        headers,
        body: String::from_utf8(body).unwrap(),
    }
}

fn write_response(mut stream: TcpStream, status: u16, headers: &[(&str, String)], body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\n",
        body.len()
    )
    .unwrap();
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    write!(stream, "\r\n{body}").unwrap();
    stream.flush().unwrap();
}

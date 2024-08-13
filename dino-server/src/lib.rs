mod config;
mod engine;
mod error;
mod middleware;
mod router;

use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Host, Query, State},
    http::{request::Parts, Response},
    response::IntoResponse,
    routing::any,
    Router,
};
use dashmap::DashMap;
use indexmap::IndexMap;
use matchit::Match;
use middleware::ServerTimeLayer;
use std::collections::HashMap;
use tokio::net::TcpListener;
use tracing::info;

use axum::extract::Request;
pub use config::*;
pub use engine::*;
pub use error::AppError;
pub use router::*;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::sync::mpsc::channel;
use tokio::sync::{mpsc, oneshot, Mutex};

type ProjectRoutes = IndexMap<String, Vec<ProjectRoute>>;

#[derive(Clone)]
pub struct AppState {
    // key is hostname
    routers: DashMap<String, SwappableAppRouter>,
}

#[derive(Clone)]
pub struct TenentRouter {
    host: String,
    router: SwappableAppRouter,
}

pub async fn start_server(port: u16, routers: Vec<TenentRouter>) -> Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(addr).await?;

    info!("listening on {}", listener.local_addr()?);

    let map = DashMap::new();
    for TenentRouter { host, router } in routers {
        map.insert(host, router);
    }
    let state = AppState::new(map);
    let app = Router::new()
        .route("/*path", any(handler))
        .layer(ServerTimeLayer)
        .with_state(state);

    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

// try
// Worker Pool Definition
struct WorkerPool {
    sender: mpsc::Sender<(Req, oneshot::Sender<String>)>,
}

impl WorkerPool {
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<(Req, oneshot::Sender<String>)>(100);
        let receiver = Arc::new(Mutex::new(receiver)); // Arc<Mutex<Receiver>>

        for _ in 0..size {
            let receiver_clone = Arc::clone(&receiver); // Clone the Arc, not the Receiver
            let sender_clone = sender.clone(); // To send messages to the channel

            tokio::spawn(async move {
                while let Some((req, responder)) = receiver_clone.lock().await.recv().await {
                    // Handle the request and send the response
                    let res = handle_request(req).await;
                    let _ = responder.send(res);
                }
            });
        }

        Self { sender }
    }

    pub async fn send(&self, req: Req) -> String {
        let (responder, receiver) = oneshot::channel();
        let _ = self.sender.send((req, responder)).await;
        receiver.await.unwrap() // Wait for the response
    }
}
async fn handle_request(req: Req) -> String {
    // Simulate processing and return response
    format!("Handled request: {:?}", req)
}
// we only support JSON requests and return JSON responses
#[allow(unused)]
async fn handler(
    State(state): State<AppState>,
    parts: Parts,
    Host(mut host): Host,
    Query(query): Query<HashMap<String, String>>,
    body: Option<Bytes>,
) -> Result<Response<String>, AppError> {
    let router = get_router_by_host(host, state)?;
    let matched = router.match_it(parts.method.clone(), parts.uri.path())?;
    let req = assemble_req(&matched, &parts, query, body)?;
    //let handler = matched.value;
    // TODO: build a worker pool, and send req via mpsc channel and get res from oneshot channel
    //Ok(Response::from(res))
    // Use a worker pool to process the request and wait for the response
    let worker_pool = WorkerPool::new(4); // Create a worker pool with 4 workers
    let response_body = worker_pool.send(req).await; // Send the request to the worker pool and await the response

    // Return the response to the client
    Ok(Response::<String>::new(response_body.into()))
}

impl AppState {
    pub fn new(routers: DashMap<String, SwappableAppRouter>) -> Self {
        Self { routers }
    }
}

impl TenentRouter {
    pub fn new(host: impl Into<String>, router: SwappableAppRouter) -> Self {
        Self {
            host: host.into(),
            router,
        }
    }
}

fn get_router_by_host(mut host: String, state: AppState) -> Result<AppRouter, AppError> {
    let _ = host.split_off(host.find(':').unwrap_or(host.len()));
    info!("host: {:?}", host);

    let router: AppRouter = state
        .routers
        .get(&host)
        .ok_or(AppError::HostNotFound(host))?
        .load();

    Ok(router)
}

fn assemble_req(
    matched: &Match<&str>,
    parts: &Parts,
    query: HashMap<String, String>,
    body: Option<Bytes>,
) -> Result<Req, AppError> {
    let params: HashMap<String, String> = matched
        .params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    // convert request data into Req
    let headers = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();
    let body = body.and_then(|v| String::from_utf8(v.into()).ok());

    let req = Req::builder()
        .method(parts.method.to_string())
        .url(parts.uri.to_string())
        .query(query)
        .params(params)
        .headers(headers)
        .body(body)
        .build();

    Ok(req)
}

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

use axum::{
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Router, Server,
};
use parking_lot::{const_rwlock, Condvar, Mutex, RwLock};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::spawn;

type OnceMutexWithCondvar = OnceLock<Arc<(Mutex<Vec<(String, String)>>, Condvar)>>;

static QUEUE: OnceMutexWithCondvar = OnceLock::new();
static RESULT: RwLock<Vec<(String, String)>> = const_rwlock(Vec::new());
static COUNT: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() {
    QUEUE.get_or_init(|| Arc::new((Mutex::new(Vec::new()), Condvar::new())));
    let router = Router::new()
        .route("/:data", post(queue))
        .route("/fetchResult/:req_number", get(fetch_result))
        .into_make_service();
    let addr = SocketAddr::from(([0, 0, 0, 0], 13700));
    println!("Listening on {addr}");
    spawn(async move {
        let queue = QUEUE.get().unwrap().clone();
        let (lock, condvar) = &*queue;
        let queue_value = &mut lock.lock();
        loop {
            condvar.wait(queue_value);
            while let Some(data) = queue_value.pop() {
                COUNT.fetch_add(1, Ordering::Release);
                process(data);
            }
        }
    });
    Server::bind(&addr)
        .serve(router)
        .await
        .expect("Server startup failed.");
}

async fn queue(Path(data): Path<String>) -> StatusCode {
    let queue = QUEUE.get().unwrap().clone();
    let (lock, condvar) = &*queue;
    let req_number = format!(
        "{}-{}",
        OffsetDateTime::now_utc().unix_timestamp(),
        lock.lock().len()
    );
    lock.lock().push((req_number, data));
    condvar.notify_one();
    StatusCode::ACCEPTED
}

async fn fetch_result(Path(req_number): Path<String>) -> (StatusCode, String) {
    let queue = QUEUE.get().unwrap().0.lock();
    let find_queue = queue.iter().find(|(key, _)| key == &req_number);
    let result = RESULT.read();
    let find_result = result.iter().find(|(key, _)| key == &req_number);
    match (find_queue, find_result) {
        (Some(_), None) => (
            StatusCode::ACCEPTED,
            COUNT.load(Ordering::Relaxed).to_string(),
        ),
        (None, Some((_, _))) => (StatusCode::OK, COUNT.load(Ordering::Relaxed).to_string()),
        (None, None) => (
            StatusCode::NOT_FOUND,
            COUNT.load(Ordering::Relaxed).to_string(),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            COUNT.load(Ordering::Relaxed).to_string(),
        ),
    }
}

fn process((req_number, _data): (String, String)) {
    let mut result = RESULT.write();
    result.push((
        req_number,
        OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
    ));
}

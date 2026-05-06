use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use url::Url;

pub struct MockServer {
    base_url: Url,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    pub async fn spawn(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_task = requests.clone();

        tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0_u8; 8192];
                let mut raw = Vec::new();

                loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                requests_for_task
                    .lock()
                    .await
                    .push(String::from_utf8_lossy(&raw).to_string());
                socket
                    .write_all(response.to_http().as_bytes())
                    .await
                    .unwrap();
                socket.shutdown().await.unwrap();
            }
        });

        Self {
            base_url: Url::parse(&format!("http://{addr}/")).unwrap(),
            requests,
        }
    }

    pub fn url(&self, path: &str) -> Url {
        self.base_url.join(path.trim_start_matches('/')).unwrap()
    }

    pub async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }
}

pub struct MockResponse {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

impl MockResponse {
    pub fn json(status: u16, headers: Vec<(&'static str, &'static str)>, body: &str) -> Self {
        Self {
            status,
            headers,
            body: body.to_string(),
        }
    }

    fn to_http(&self) -> String {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "OK",
        };

        let mut response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            reason,
            self.body.len()
        );
        for (name, value) in &self.headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(&self.body);
        response
    }
}

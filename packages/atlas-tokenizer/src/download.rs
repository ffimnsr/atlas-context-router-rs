//! HTTPS asset download and sha256 verification.
//!
//! Downloads are HTTPS-only at the initial URL and after redirects, use a
//! 10-second connect timeout and 60-second total timeout, stream at most
//! [`MAX_DOWNLOAD_BYTES`], and verify sha256 before bytes are returned. No
//! authorization headers, custom headers, or query secrets are added.

use std::io::Read;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;

use crate::assets::sha256_hex;
use crate::error::{Error, Result};

/// Maximum accepted asset download size.
pub const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `AllowHttp` is only constructed by unit tests.
enum SchemePolicy {
    HttpsOnly,
    AllowHttp,
}

/// Fetch an HTTPS URL and verify its sha256 digest before returning bytes.
pub fn fetch_verified(url: &str, expected_sha256: &str) -> Result<Vec<u8>> {
    fetch_with_policy(url, expected_sha256, SchemePolicy::HttpsOnly)
}

fn fetch_with_policy(url: &str, expected_sha256: &str, policy: SchemePolicy) -> Result<Vec<u8>> {
    let allow_http = matches!(policy, SchemePolicy::AllowHttp);
    let parsed = Url::parse(url).map_err(|err| Error::DownloadRequest {
        url: url.to_owned(),
        message: err.to_string(),
    })?;
    if parsed.scheme() != "https" && !allow_http {
        return Err(Error::InsecureDownloadUrl {
            url: url.to_owned(),
        });
    }

    let redirect_policy = if allow_http {
        Policy::limited(10)
    } else {
        Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                attempt.error("redirect to non-HTTPS URL")
            } else {
                attempt.follow()
            }
        })
    };
    let mut builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .redirect(redirect_policy);
    if !allow_http {
        builder = builder.https_only(true);
    }
    let client = builder.build().map_err(|err| Error::DownloadRequest {
        url: url.to_owned(),
        message: err.to_string(),
    })?;

    let response = client
        .get(parsed)
        .send()
        .map_err(|err| Error::DownloadRequest {
            url: url.to_owned(),
            message: err.to_string(),
        })?;
    if response.url().scheme() != "https" && !allow_http {
        return Err(Error::InsecureRedirect {
            url: response.url().to_string(),
        });
    }
    if !response.status().is_success() {
        return Err(Error::HttpStatus {
            url: url.to_owned(),
            status: response.status().as_u16(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES as u64)
    {
        return Err(Error::DownloadTooLarge {
            limit: MAX_DOWNLOAD_BYTES,
        });
    }

    let mut bytes = Vec::new();
    let mut limited = response.take(MAX_DOWNLOAD_BYTES as u64 + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|err| Error::DownloadRequest {
            url: url.to_owned(),
            message: err.to_string(),
        })?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(Error::DownloadTooLarge {
            limit: MAX_DOWNLOAD_BYTES,
        });
    }

    let actual = sha256_hex(&bytes);
    if actual != expected_sha256 {
        return Err(Error::HashMismatch {
            expected: expected_sha256.to_owned(),
            actual,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::*;

    type Handler = Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>;

    struct FixtureServer {
        addr: SocketAddr,
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    impl FixtureServer {
        fn spawn(handler: Handler) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
            let addr = listener.local_addr().expect("addr");
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        break;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buffer = [0u8; 8192];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_owned();
                    let response = handler(&path);
                    let _ = stream.write_all(&response);
                }
            });
            Self {
                addr,
                stop,
                handle: Some(handle),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.addr)
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.addr);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn response(status: &str, body: &[u8], extra_headers: &str) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    #[test]
    fn fetches_and_verifies_body() {
        let server = FixtureServer::spawn(Arc::new(|_| response("200 OK", b"hello", "")));
        let expected = sha256_hex(b"hello");
        let bytes = fetch_with_policy(&server.url("/asset"), &expected, SchemePolicy::AllowHttp)
            .expect("fetch");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn rejects_hash_mismatch() {
        let server = FixtureServer::spawn(Arc::new(|_| response("200 OK", b"hello", "")));
        let err = fetch_with_policy(
            &server.url("/asset"),
            &"0".repeat(64),
            SchemePolicy::AllowHttp,
        )
        .expect_err("hash mismatch");
        assert!(matches!(err, Error::HashMismatch { .. }));
    }

    #[test]
    fn rejects_oversized_content_length_before_reading() {
        let server = FixtureServer::spawn(Arc::new(|_| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_DOWNLOAD_BYTES + 1
            )
            .into_bytes()
        }));
        let err = fetch_with_policy(
            &server.url("/asset"),
            &"0".repeat(64),
            SchemePolicy::AllowHttp,
        )
        .expect_err("too large");
        assert!(matches!(err, Error::DownloadTooLarge { .. }));
    }

    #[test]
    fn follows_redirect_for_fixture_policy() {
        let server = FixtureServer::spawn(Arc::new(|path| {
            if path == "/start" {
                response("302 Found", b"", "Location: /final\r\n")
            } else {
                response("200 OK", b"redirected", "")
            }
        }));
        let expected = sha256_hex(b"redirected");
        let bytes = fetch_with_policy(&server.url("/start"), &expected, SchemePolicy::AllowHttp)
            .expect("fetch");
        assert_eq!(bytes, b"redirected");
    }

    #[test]
    fn rejects_plain_http_for_production_policy() {
        let err = fetch_with_policy(
            "http://127.0.0.1:1/asset",
            &"0".repeat(64),
            SchemePolicy::HttpsOnly,
        )
        .expect_err("insecure URL");
        assert!(matches!(err, Error::InsecureDownloadUrl { .. }));
    }
}

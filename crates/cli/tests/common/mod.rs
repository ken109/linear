//! Shared helpers for the integration tests: a scratch config dir, a mock
//! Linear server and a way to run the `linear` binary against them.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Reply {
    pub status: u16,
    pub body: String,
}

pub fn ok(body: &str) -> Reply {
    Reply {
        status: 200,
        body: body.to_owned(),
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub authorization: Option<String>,
    pub body: String,
}

/// A one-thread HTTP server that answers each request with the next queued reply.
pub struct Mock {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    pub fn start(replies: Vec<Reply>) -> Mock {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let url = format!(
            "http://{}/graphql",
            server.server_addr().to_ip().expect("ip")
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let mut queue: VecDeque<Reply> = replies.into();
        thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                let authorization = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("authorization"))
                    .map(|h| h.value.to_string());
                log.lock().unwrap().push(Seen {
                    authorization,
                    body,
                });
                let reply = queue.pop_front().unwrap_or(Reply {
                    status: 500,
                    body: "{\"errors\":[{\"message\":\"mock: no reply queued\"}]}".into(),
                });
                let response = tiny_http::Response::from_string(reply.body)
                    .with_status_code(reply.status)
                    .with_header(
                        tiny_http::Header::from_bytes("content-type", "application/json").unwrap(),
                    );
                let _ = req.respond(response);
            }
        });
        Mock { url, seen }
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// An isolated config directory and working directory.
pub struct Sandbox {
    pub root: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        Sandbox {
            root: tempfile::tempdir().expect("tempdir"),
        }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.path().join("config")
    }

    pub fn cwd(&self) -> PathBuf {
        let d = self.root.path().join("work");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Run `linear` with a clean environment pointing at this sandbox.
    pub fn run(&self, args: &[&str], mock: Option<&Mock>, extra_env: &[(&str, &str)]) -> Output {
        self.run_stdin(args, mock, extra_env, None)
    }

    pub fn run_stdin(
        &self,
        args: &[&str],
        mock: Option<&Mock>,
        extra_env: &[(&str, &str)],
        stdin: Option<&str>,
    ) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_linear"));
        cmd.args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.root.path())
            .env("LINEAR_CONFIG_DIR", self.config_dir())
            .current_dir(self.cwd());
        if let Some(m) = mock {
            cmd.env("LINEAR_API_URL", &m.url);
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn linear");
        {
            use std::io::Write;
            let mut si = child.stdin.take().unwrap();
            if let Some(text) = stdin {
                let _ = si.write_all(text.as_bytes());
            }
        }
        child.wait_with_output().expect("wait")
    }
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

pub fn code(o: &Output) -> i32 {
    o.status.code().expect("exit code")
}

pub const WHOAMI_OK: &str = r#"{"data":{"viewer":{"id":"00000000-0000-4000-8000-000000000001","name":"Alice Example","displayName":"alice","email":"alice@example.com","active":true,"isMe":true},"organization":{"id":"00000000-0000-4000-8000-000000000002","name":"Example","urlKey":"example"}}}"#;

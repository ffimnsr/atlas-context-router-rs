use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use atlas_parser::ParserRegistry;

// ---------------------------------------------------------------------------
// Fixture source snippets used across all parser benchmarks
// ---------------------------------------------------------------------------

const RUST_SRC: &str = r#"
use std::collections::HashMap;
use std::sync::Arc;

/// Registry holds named byte payloads.
pub struct Registry {
    entries: HashMap<String, Vec<u8>>,
    version: u32,
}

impl Registry {
    pub fn new() -> Self {
        Self { entries: HashMap::new(), version: 0 }
    }

    pub fn insert(&mut self, key: String, value: Vec<u8>) {
        self.entries.insert(key, value);
        self.version += 1;
    }

    pub fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.entries.remove(key)
    }
}

pub trait Store: Send + Sync {
    fn put(&mut self, key: &str, value: &[u8]);
    fn fetch(&self, key: &str) -> Option<Arc<Vec<u8>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut r = Registry::new();
        r.insert("k".into(), vec![1, 2]);
        assert!(r.get("k").is_some());
    }

    #[test]
    fn test_remove() {
        let mut r = Registry::new();
        r.insert("k".into(), vec![1]);
        assert!(r.remove("k").is_some());
        assert!(r.get("k").is_none());
    }
}
"#;

const GO_SRC: &str = r#"
package server

import (
    "fmt"
    "net/http"
)

type Handler struct {
    prefix string
    mux    *http.ServeMux
}

func NewHandler(prefix string) *Handler {
    return &Handler{
        prefix: prefix,
        mux:    http.NewServeMux(),
    }
}

func (h *Handler) Register(path string, fn http.HandlerFunc) {
    h.mux.HandleFunc(h.prefix+path, fn)
}

func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
    h.mux.ServeHTTP(w, r)
}

func HealthCheck(w http.ResponseWriter, r *http.Request) {
    fmt.Fprintln(w, "ok")
}

func TestNewHandler(t *testing.T) {
    h := NewHandler("/api")
    if h == nil {
        t.Fatal("expected handler")
    }
}
"#;

const PYTHON_SRC: &str = r#"
import os
import sys
from pathlib import Path
from typing import Optional, List

class FileScanner:
    """Scan a directory recursively."""

    def __init__(self, root: str, max_size: int = 10 * 1024 * 1024):
        self.root = Path(root)
        self.max_size = max_size

    def scan(self) -> List[Path]:
        results = []
        for entry in self.root.rglob("*"):
            if entry.is_file() and entry.stat().st_size <= self.max_size:
                results.append(entry)
        return results

    def find(self, name: str) -> Optional[Path]:
        for path in self.scan():
            if path.name == name:
                return path
        return None


def hash_file(path: str) -> str:
    import hashlib
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def test_hash_file(tmp_path):
    p = tmp_path / "a.txt"
    p.write_text("hello")
    digest = hash_file(str(p))
    assert len(digest) == 64
"#;

const TS_SRC: &str = r#"
import { EventEmitter } from 'events';

export interface Config {
    host: string;
    port: number;
    timeout?: number;
}

export type Handler = (req: Request, res: Response) => void;

export enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

export class Server extends EventEmitter {
    private config: Config;
    private running: boolean = false;

    constructor(config: Config) {
        super();
        this.config = config;
    }

    start(): Promise<void> {
        this.running = true;
        this.emit('start');
        return Promise.resolve();
    }

    stop(): void {
        this.running = false;
        this.emit('stop');
    }

    isRunning(): boolean {
        return this.running;
    }
}

export function createServer(config: Config): Server {
    return new Server(config);
}
"#;

// ---------------------------------------------------------------------------
// Benchmark functions
// ---------------------------------------------------------------------------

fn bench_parse_rust(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    c.bench_with_input(BenchmarkId::new("parse", "rust"), &RUST_SRC, |b, src| {
        b.iter(|| registry.parse("src/registry.rs", "abc123", src.as_bytes()));
    });
}

fn bench_parse_go(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    c.bench_with_input(BenchmarkId::new("parse", "go"), &GO_SRC, |b, src| {
        b.iter(|| registry.parse("server/handler.go", "abc123", src.as_bytes()));
    });
}

fn bench_parse_python(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    c.bench_with_input(BenchmarkId::new("parse", "python"), &PYTHON_SRC, |b, src| {
        b.iter(|| registry.parse("scanner.py", "abc123", src.as_bytes()));
    });
}

fn bench_parse_typescript(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    c.bench_with_input(BenchmarkId::new("parse", "typescript"), &TS_SRC, |b, src| {
        b.iter(|| registry.parse("src/server.ts", "abc123", src.as_bytes()));
    });
}

fn bench_parse_all_languages(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    let files: &[(&str, &str)] = &[
        ("src/registry.rs", RUST_SRC),
        ("server/handler.go", GO_SRC),
        ("scanner.py", PYTHON_SRC),
        ("src/server.ts", TS_SRC),
    ];
    c.bench_function("parse/all_languages", |b| {
        b.iter(|| {
            for (path, src) in files {
                let _ = registry.parse(path, "abc123", src.as_bytes());
            }
        });
    });
}

criterion_group!(
    benches,
    bench_parse_rust,
    bench_parse_go,
    bench_parse_python,
    bench_parse_typescript,
    bench_parse_all_languages,
);
criterion_main!(benches);
